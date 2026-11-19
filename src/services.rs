//! Scoped Flatpak operations, catalogue metadata and local Store state.
//!
//! Mutations use explicit installation scopes and the shared permission authority.
//! Metadata uses bounded HTTP transport. A missing Flatpak executable is reported
//! as a failure; production never substitutes the fixture catalogue.

use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) mod flatpak_failure;
pub(crate) mod install_review;
mod local_icons;
pub(crate) mod permission_failure;
mod process;

use enzui_services::process_transport;
use flatpak_failure::{FlatpakCommandFailure, TransportFailure};
pub(crate) use permission_failure::{PermissionAction, PermissionActionFailure};

include!("services/fixtures.rs");
include!("services/scope.rs");
include!("services/authz.rs");
include!("services/mutation.rs");
include!("services/updates.rs");
include!("services/entries.rs");
include!("services/executor.rs");
include!("services/launch.rs");
include!("services/reads.rs");
include!("services/updates_scan.rs");
include!("services/progress.rs");
include!("services/remotes_mgmt.rs");
include!("services/permissions.rs");
mod shared_permissions;

// Tests isolate fallback storage from the account's real Store data.
#[cfg(test)]
pub(crate) fn dirs_fallback() -> std::path::PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map_or_else(test_data_root, std::path::PathBuf::from);
    base.join("store-app")
}

// Process-scoped temporary storage preserves restart tests without reaching host user data.
#[cfg(test)]
pub(crate) fn test_data_root() -> std::path::PathBuf {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| crate::test_scratch::path("datadir"))
        .clone()
}

/// Maximum retained recent searches, ordered newest first.
pub(crate) const RECENT_SEARCHES_LIMIT: usize = 8;

/// Maximum UTF-8 bytes retained for one recent-search query.
pub(crate) const RECENT_SEARCH_MAX_BYTES: usize = 256;

/// Maximum Unicode scalar values retained for one recent-search query.
pub(crate) const RECENT_SEARCH_MAX_SCALARS: usize = 128;

/// Maximum physical lines admitted before the whole file is refused.
pub(crate) const RECENT_SEARCHES_MAX_LINES: usize = RECENT_SEARCHES_LIMIT;

/// Documented complete-file byte cap: every admitted line at its maximum plus a
/// newline terminator. The reader admits this many bytes and refuses one more.
pub(crate) const RECENT_SEARCHES_FILE_MAX_BYTES: u64 =
    (RECENT_SEARCHES_MAX_LINES as u64) * (RECENT_SEARCH_MAX_BYTES as u64 + 1);

/// The recent-searches file under a given data directory, for tests that
/// round-trip through a temp directory. Production resolves the file through
/// [`crate::roots::DataCapability::recent_searches_path`] under the validated
/// data root.
#[cfg(test)]
pub(crate) fn recent_searches_path_at(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("recent-searches.txt")
}

/// A recent-search query that a bounded limit refused, carrying no pathname.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecentSearchRefusal {
    /// The trimmed query's UTF-8 length exceeds [`RECENT_SEARCH_MAX_BYTES`].
    TooManyBytes { bytes: usize },
    /// The trimmed query's scalar count exceeds [`RECENT_SEARCH_MAX_SCALARS`].
    TooManyScalars { scalars: usize },
}

impl RecentSearchRefusal {
    /// A short, path-free message suitable for the search UI.
    #[must_use]
    pub(crate) fn user_summary(&self) -> String {
        match self {
            Self::TooManyBytes { .. } => {
                format!("That search is too long to keep (limit {RECENT_SEARCH_MAX_BYTES} bytes).")
            }
            Self::TooManyScalars { .. } => format!(
                "That search is too long to keep (limit {RECENT_SEARCH_MAX_SCALARS} characters)."
            ),
        }
    }
}

/// Why the recent-search history was refused whole, carrying no pathname.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum RecentSearchesUnavailableKind {
    /// The path could not be opened as a regular file (symlink, FIFO, device).
    OpenRefused,
    /// Reading the descriptor or its metadata failed.
    ReadFailed,
    /// The complete file exceeds [`RECENT_SEARCHES_FILE_MAX_BYTES`].
    TooLarge,
    /// The file grew past the admitted length during the read.
    GrewDuringRead,
    /// The descriptor's identity or length changed across the read.
    Replaced,
    /// More physical lines than [`RECENT_SEARCHES_MAX_LINES`].
    TooManyLines,
    /// A line exceeds the per-query byte or scalar limit.
    MalformedLine,
    /// A line is not valid UTF-8.
    InvalidUtf8,
}

/// A path-free reason the recent-search history could not be trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecentSearchesUnavailable {
    kind: RecentSearchesUnavailableKind,
}

impl RecentSearchesUnavailable {
    /// A short, path-free message suitable for the recovery banner.
    #[must_use]
    pub(crate) fn user_summary(self) -> &'static str {
        "Recent searches couldn't be read and were left untouched."
    }
}

/// Outcome of reading the recent-search history from disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecentSearchesLoad {
    /// A trusted history, newest first; empty for a missing or empty file.
    Loaded(Vec<String>),
    /// The file was refused whole and left byte-identical on disk.
    Unavailable(RecentSearchesUnavailable),
}

impl RecentSearchesLoad {
    /// The loaded history, or an empty list when unavailable.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn into_history(self) -> Vec<String> {
        match self {
            Self::Loaded(history) => history,
            Self::Unavailable(_) => Vec::new(),
        }
    }
}

// ASCII case fold keeps dedup bounded and matches the queries the search box
// admits; a non-ASCII duplicate in two cases is kept as two entries.
fn recent_search_fold(query: &str) -> String {
    query.to_ascii_lowercase()
}

// Enforce the shared per-query byte and scalar limits before allocating a
// canonical query. The single gate the loader, `push`, and serialization share.
fn canonical_recent_search(trimmed: &str) -> Result<String, RecentSearchRefusal> {
    if trimmed.len() > RECENT_SEARCH_MAX_BYTES {
        return Err(RecentSearchRefusal::TooManyBytes {
            bytes: trimmed.len(),
        });
    }
    let scalars = trimmed.chars().count();
    if scalars > RECENT_SEARCH_MAX_SCALARS {
        return Err(RecentSearchRefusal::TooManyScalars { scalars });
    }
    Ok(trimmed.to_owned())
}

// Yield each physical line without the trailing-newline terminator, so a file
// ending in `\n` does not present a spurious final empty line.
fn recent_search_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = bytes;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        match rest.iter().position(|&byte| byte == b'\n') {
            Some(index) => {
                let (line, tail) = rest.split_at(index);
                rest = &tail[1..];
                Some(line)
            }
            None => {
                let line = rest;
                rest = &[];
                Some(line)
            }
        }
    })
}

fn decode_recent_searches(bytes: &[u8]) -> Result<Vec<String>, RecentSearchesUnavailableKind> {
    use RecentSearchesUnavailableKind as Kind;

    let mut history: Vec<String> = Vec::new();
    let mut retained_bytes: usize = 0;
    let mut physical_lines: usize = 0;
    for segment in recent_search_lines(bytes) {
        physical_lines += 1;
        if physical_lines > RECENT_SEARCHES_MAX_LINES {
            return Err(Kind::TooManyLines);
        }
        // Reject an oversized line by its raw length before decoding it, so an
        // oversized line is never materialized as a `str` or `String`.
        if segment.len() > RECENT_SEARCH_MAX_BYTES {
            return Err(Kind::MalformedLine);
        }
        let text = std::str::from_utf8(segment).map_err(|_| Kind::InvalidUtf8)?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let canonical = canonical_recent_search(trimmed).map_err(|_| Kind::MalformedLine)?;
        let folded = recent_search_fold(&canonical);
        if history
            .iter()
            .any(|existing| recent_search_fold(existing) == folded)
        {
            continue;
        }
        if history.len() >= RECENT_SEARCHES_LIMIT {
            continue;
        }
        let total = retained_bytes
            .checked_add(canonical.len())
            .ok_or(Kind::TooLarge)?;
        match u64::try_from(total) {
            Ok(aggregate) if aggregate <= RECENT_SEARCHES_FILE_MAX_BYTES => {}
            _ => return Err(Kind::TooLarge),
        }
        retained_bytes = total;
        history.push(canonical);
    }
    Ok(history)
}

// The observed device and inode of a present recent-search file, or `None` for
// a verifiably absent one. Compared against the publication authority's own
// identity so the read and the authority are bound to one inode.
type RecentSearchIdentity = Option<(u64, u64)>;

#[cfg(test)]
thread_local! {
    static AFTER_METADATA_HOOK: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
    static AFTER_READ_HOOK: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_after_metadata_hook() {
    if let Some(mut hook) = AFTER_METADATA_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
fn run_after_read_hook() {
    if let Some(mut hook) = AFTER_READ_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

fn read_recent_searches_bounded(
    path: &std::path::Path,
) -> Result<(Vec<String>, RecentSearchIdentity), RecentSearchesUnavailableKind> {
    use RecentSearchesUnavailableKind as Kind;
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;

    let mut file = match enzui_core::atomic_state::open_state_file_nofollow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), None));
        }
        Err(_) => return Err(Kind::OpenRefused),
    };
    let before = file.metadata().map_err(|_| Kind::ReadFailed)?;
    if !before.is_file() {
        return Err(Kind::OpenRefused);
    }
    if before.len() > RECENT_SEARCHES_FILE_MAX_BYTES {
        return Err(Kind::TooLarge);
    }
    let identity = (before.dev(), before.ino());

    #[cfg(test)]
    run_after_metadata_hook();

    let mut buffer = Vec::new();
    // Admit one byte past the cap: reading it proves the file grew and the
    // whole document is refused rather than silently truncated.
    (&mut file)
        .take(RECENT_SEARCHES_FILE_MAX_BYTES + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| Kind::ReadFailed)?;
    let read_len = u64::try_from(buffer.len()).map_err(|_| Kind::TooLarge)?;
    if read_len > RECENT_SEARCHES_FILE_MAX_BYTES {
        return Err(Kind::GrewDuringRead);
    }

    #[cfg(test)]
    run_after_read_hook();

    let after = file.metadata().map_err(|_| Kind::ReadFailed)?;
    if after.dev() != before.dev()
        || after.ino() != before.ino()
        || after.len() != before.len()
        || after.len() != read_len
    {
        return Err(Kind::Replaced);
    }
    Ok((decode_recent_searches(&buffer)?, Some(identity)))
}

/// Read and validate the recent-search history through a nofollow, nonblocking
/// regular descriptor, refusing the whole document on any uncertainty.
///
/// A missing file is a trusted empty history. A symlink, FIFO, oversized,
/// grown, replaced, or malformed file yields [`RecentSearchesLoad::Unavailable`]
/// and is left byte-identical on disk. This does not bind a publication
/// authority; use [`acquire_recent_searches`] for the boot and retry paths.
#[must_use]
#[cfg(test)]
pub(crate) fn read_recent_searches(path: &std::path::Path) -> RecentSearchesLoad {
    match read_recent_searches_bounded(path) {
        Ok((history, _)) => RecentSearchesLoad::Loaded(history),
        Err(kind) => RecentSearchesLoad::Unavailable(RecentSearchesUnavailable { kind }),
    }
}

/// The history plus the publication authority bound to the exact inode it was
/// read from.
///
/// The authority is `Some` only when the read succeeded and its inode equals the
/// authority's retained identity, so a later automatic save publishes through
/// that descriptor rather than re-deriving one from the pathname.
pub(crate) struct RecentSearchesAcquire {
    outcome: RecentSearchesLoad,
    authority: Option<enzui_core::state_authority::StateAuthority>,
    /// The identity the authority verified for a refused read, retained so an
    /// explicit retire moves only that exact inode. `None` when no trusted read
    /// identity exists (the authority could not open the destination).
    expected: Option<enzui_core::quarantine::ExpectedSourceIdentity>,
}

impl RecentSearchesAcquire {
    /// The bounded read outcome.
    #[must_use]
    pub(crate) fn outcome(&self) -> &RecentSearchesLoad {
        &self.outcome
    }

    fn into_parts(
        self,
    ) -> (
        RecentSearchesLoad,
        Option<enzui_core::state_authority::StateAuthority>,
        Option<enzui_core::quarantine::ExpectedSourceIdentity>,
    ) {
        (self.outcome, self.authority, self.expected)
    }
}

/// Open the recent-search file's identity-bound publication authority and read
/// its history through the same verified inode.
///
/// The authority retains the existing inode (or the parent, for an absent file)
/// by descriptor. A read whose inode differs from the authority's retained
/// identity is a substitution and refuses without binding an authority.
#[must_use]
pub(crate) fn acquire_recent_searches(path: &std::path::Path) -> RecentSearchesAcquire {
    use enzui_core::quarantine::ExpectedSourceIdentity;
    use enzui_core::state_authority::StateAuthority;

    let authority = match StateAuthority::open_or_create_tightening(path) {
        Ok(authority) => authority,
        Err(_) => {
            return RecentSearchesAcquire {
                outcome: RecentSearchesLoad::Unavailable(RecentSearchesUnavailable {
                    kind: RecentSearchesUnavailableKind::OpenRefused,
                }),
                authority: None,
                expected: None,
            };
        }
    };
    let identity = authority.identity();
    let expected_pair = identity.map(|identity| (identity.device(), identity.inode()));
    // The exact inode the authority validated, retained so a later retire moves
    // only this file and refuses a replacement raced in after the read.
    let expected = identity.map(|id| ExpectedSourceIdentity::from_dev_ino(id.device(), id.inode()));
    match read_recent_searches_bounded(path) {
        Ok((history, observed)) if observed == expected_pair => RecentSearchesAcquire {
            outcome: RecentSearchesLoad::Loaded(history),
            authority: Some(authority),
            expected: None,
        },
        Ok(_) => RecentSearchesAcquire {
            outcome: RecentSearchesLoad::Unavailable(RecentSearchesUnavailable {
                kind: RecentSearchesUnavailableKind::Replaced,
            }),
            authority: None,
            expected,
        },
        Err(kind) => RecentSearchesAcquire {
            outcome: RecentSearchesLoad::Unavailable(RecentSearchesUnavailable { kind }),
            authority: None,
            expected,
        },
    }
}

/// Fold a freshly-run query into an existing history, enforcing the same
/// per-query limits the loader and serializer enforce.
///
/// Newest first; case-insensitive dedup with the new query's casing winning;
/// truncated to [`RECENT_SEARCHES_LIMIT`]. A blank query is a no-op.
///
/// # Errors
/// Returns [`RecentSearchRefusal`] when the trimmed query exceeds a limit,
/// leaving the caller to surface it and keep the history unchanged.
pub(crate) fn push_recent_search_checked(
    mut history: Vec<String>,
    query: &str,
) -> Result<Vec<String>, RecentSearchRefusal> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(history);
    }
    let canonical = canonical_recent_search(trimmed)?;
    let folded = recent_search_fold(&canonical);
    history.retain(|existing| recent_search_fold(existing) != folded);
    history.insert(0, canonical);
    history.truncate(RECENT_SEARCHES_LIMIT);
    Ok(history)
}

/// Serialize a history to the on-disk line format, enforcing the per-query
/// limits so a save never writes an oversized or malformed line.
///
/// # Errors
/// Returns [`RecentSearchRefusal`] for the first query that exceeds a limit.
pub(crate) fn serialize_recent_searches(
    history: &[String],
) -> Result<Vec<u8>, RecentSearchRefusal> {
    let mut out = Vec::new();
    for query in history.iter().take(RECENT_SEARCHES_LIMIT) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            continue;
        }
        let canonical = canonical_recent_search(trimmed)?;
        out.extend_from_slice(canonical.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

/// Read the recent-search history from an explicit file path, returning an
/// empty list when the file is missing or refused. See [`read_recent_searches`]
/// for the trusted/unavailable distinction the recovery UI needs.
#[must_use]
#[cfg(test)]
pub(crate) fn load_recent_searches_at(path: &std::path::Path) -> Vec<String> {
    read_recent_searches(path).into_history()
}

fn publish_recent_bytes_reported(
    authority: &mut enzui_core::state_authority::StateAuthority,
    bytes: &[u8],
) -> Result<enzui_core::state_authority::StateIdentity, enzui_core::state_authority::PublishError> {
    #[cfg(test)]
    if let Some(write) = TEST_RAW_WRITER.with(|slot| slot.borrow_mut().take()) {
        return authority.publish_with(|staged| write(staged, bytes));
    }
    authority.publish(bytes)
}

fn publish_recent_bytes(
    authority: &mut enzui_core::state_authority::StateAuthority,
    bytes: &[u8],
) -> Result<(), enzui_core::state_authority::PublishError> {
    publish_recent_bytes_reported(authority, bytes).map(|_| ())
}

/// Persist the recent-search history atomically through the framework's
/// identity-bound publication authority. A refused query or a failed
/// publication leaves the previous file byte-identical.
///
/// # Errors
/// Returns a short message for a refused query, a directory/open failure, or a
/// publication failure.
#[cfg(test)]
pub(crate) fn save_recent_searches_at(
    path: &std::path::Path,
    history: &[String],
) -> Result<(), String> {
    let bytes = serialize_recent_searches(history)
        .map_err(|refusal| format!("write: {}", refusal.user_summary()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    let mut authority =
        enzui_core::state_authority::StateAuthority::open_or_create_tightening(path)
            .map_err(|e| format!("open: {e}"))?;
    publish_recent_bytes(&mut authority, &bytes).map_err(|e| format!("write: {e}"))
}

/// A save error for the recent-search history, carrying no pathname.
#[derive(Debug)]
pub(crate) enum RecentSearchesSaveError {
    /// No bound authority: the read was refused or a prior publication lost
    /// custody. Automatic saves stay blocked until an explicit retire; the
    /// pathname is not re-derived.
    Blocked,
    /// A query exceeded a per-query limit.
    Refused(RecentSearchRefusal),
    /// A directory-creation or open failure.
    Io(std::io::Error),
    /// The identity-bound publication failed; its custody says what survives.
    Publish(enzui_core::state_authority::PublishError),
    /// The unrecognized file could not be moved aside, so no fresh store was
    /// authorized and the previous file was left in place. Retryable.
    Quarantine,
}

impl RecentSearchesSaveError {
    /// A short, path-free message suitable for the search UI.
    #[must_use]
    pub(crate) fn user_summary(&self) -> String {
        use enzui_core::state_authority::PublishCustody;
        match self {
            Self::Blocked => {
                "Recent searches are paused until you retry or retire them.".to_owned()
            }
            Self::Refused(refusal) => refusal.user_summary(),
            Self::Io(error) => format!("Recent searches couldn't be saved ({:?}).", error.kind()),
            Self::Publish(error) => match error.custody() {
                PublishCustody::Lost => {
                    "Recent searches couldn't be saved and the file is unrecognized.".to_owned()
                }
                PublishCustody::Unchanged | PublishCustody::Rotated => {
                    "Recent searches couldn't be saved; the previous file is unchanged.".to_owned()
                }
                // Custody the app does not yet classify is treated as
                // uncertain, not benign: report the cautious unrecognized line.
                _ => "Recent searches couldn't be saved and the file is unrecognized.".to_owned(),
            },
            Self::Quarantine => {
                "Recent searches couldn't be reset; the previous file was left in place.".to_owned()
            }
        }
    }
}

/// Owns the identity-bound publication authority for the recent-search history.
///
/// Automatic [`save`](Self::save) publishes only through the authority bound at
/// read time and never re-derives one from the pathname; a refused read or a
/// publication that loses custody permanently blocks automatic saves. Only the
/// explicit [`retire`](Self::retire) decision establishes a fresh authority.
pub(crate) struct RecentSearchesFile {
    path: std::path::PathBuf,
    authority: Option<enzui_core::state_authority::StateAuthority>,
    blocked: bool,
    /// The identity the acquire read verified for the currently refused file, so
    /// a retire moves only that exact inode. `None` when no trusted read
    /// identity exists, in which case a retire refuses without mutating a name.
    expected: Option<enzui_core::quarantine::ExpectedSourceIdentity>,
}

impl RecentSearchesFile {
    /// A file with no bound data root: automatic saves stay blocked and no
    /// retire can establish authority until the store's roots are reacquired.
    #[must_use]
    pub(crate) fn unavailable() -> Self {
        Self {
            path: std::path::PathBuf::new(),
            authority: None,
            blocked: true,
            expected: None,
        }
    }

    /// Bind the file to the boot [`acquire_recent_searches`] result: the read's
    /// authority when trusted, blocked when refused.
    #[must_use]
    pub(crate) fn from_acquire(path: std::path::PathBuf, acquire: RecentSearchesAcquire) -> Self {
        let mut file = Self {
            path,
            authority: None,
            blocked: false,
            expected: None,
        };
        file.install(acquire);
        file
    }

    /// Replace the bound authority with a fresh acquire result: trusted installs
    /// its authority and unblocks; refused blocks and drops any authority.
    pub(crate) fn install(&mut self, acquire: RecentSearchesAcquire) {
        let (outcome, authority, expected) = acquire.into_parts();
        match outcome {
            RecentSearchesLoad::Loaded(_) => {
                self.authority = authority;
                self.blocked = self.authority.is_none();
                self.expected = None;
            }
            RecentSearchesLoad::Unavailable(_) => {
                self.authority = None;
                self.blocked = true;
                // Retain the read's verified identity so a retire moves only the
                // exact refused inode, never a replacement raced in afterward.
                self.expected = expected;
            }
        }
    }

    /// Whether automatic saves are blocked (no bound authority).
    #[must_use]
    pub(crate) fn is_blocked(&self) -> bool {
        self.blocked || self.authority.is_none()
    }

    /// Automatic save through the authority bound at read time.
    ///
    /// # Errors
    /// [`RecentSearchesSaveError::Blocked`] when no authority is bound, a
    /// per-query refusal, or a publication failure. A publication that loses
    /// custody blocks all later automatic saves.
    pub(crate) fn save(&mut self, history: &[String]) -> Result<(), RecentSearchesSaveError> {
        use enzui_core::state_authority::PublishCustody;

        if self.blocked {
            return Err(RecentSearchesSaveError::Blocked);
        }
        let bytes = serialize_recent_searches(history).map_err(RecentSearchesSaveError::Refused)?;
        let Some(authority) = self.authority.as_mut() else {
            self.blocked = true;
            return Err(RecentSearchesSaveError::Blocked);
        };
        match publish_recent_bytes(authority, &bytes) {
            Ok(()) => Ok(()),
            Err(error) => {
                // Fail closed: block automatic saves on a lost custody and on
                // any custody that cannot be classified as benign, so an
                // uncertain publication never keeps saving in place.
                if !matches!(
                    error.custody(),
                    PublishCustody::Unchanged | PublishCustody::Rotated
                ) {
                    self.authority = None;
                    self.blocked = true;
                    self.expected = None;
                }
                Err(RecentSearchesSaveError::Publish(error))
            }
        }
    }

    // Retire only the refused inode verified at acquisition; replacements must remain untouched.
    // Publish fresh history only after a successful no-clobber move and create-new claim.
    pub(crate) fn retire(
        &mut self,
        history: &[String],
    ) -> Result<enzui_core::state_authority::StateIdentity, RecentSearchesSaveError> {
        use enzui_core::quarantine::{QuarantineOutcome, quarantine_sibling_expected};
        use enzui_core::state_authority::StateAuthority;

        let bytes = serialize_recent_searches(history).map_err(RecentSearchesSaveError::Refused)?;
        // With no bound data root there is no pathname to retire under.
        if self.path.as_os_str().is_empty() {
            return Err(RecentSearchesSaveError::Blocked);
        }
        // A move is authorized only by the identity the acquire read verified.
        // Without one, a same-user replacement raced in after the refused read
        // could be moved as if it were the refused bytes, so refuse here before
        // mutating any name.
        let Some(expected) = self.expected else {
            return Err(RecentSearchesSaveError::Quarantine);
        };
        // Move only that exact inode aside, never overwriting or unlinking a
        // raced entry. A mismatch, replacement, durability uncertainty, or
        // partial custody leaves the previous file in place.
        match quarantine_sibling_expected(&self.path, expected) {
            QuarantineOutcome::Moved { .. } => {}
            other => {
                eprintln!(
                    "recent-search retire: quarantine did not move the exact refused file: {other:?}"
                );
                self.authority = None;
                self.blocked = true;
                return Err(RecentSearchesSaveError::Quarantine);
            }
        }
        // The original name is now free; claim it as a fresh store and refuse to
        // adopt any entry raced back into it before the first publication.
        let mut authority =
            StateAuthority::create_new(&self.path).map_err(RecentSearchesSaveError::Io)?;
        match publish_recent_bytes_reported(&mut authority, &bytes) {
            Ok(identity) => {
                self.authority = Some(authority);
                self.blocked = false;
                self.expected = None;
                Ok(identity)
            }
            Err(error) => {
                self.authority = None;
                self.blocked = true;
                Err(RecentSearchesSaveError::Publish(error))
            }
        }
    }
}

#[cfg(test)]
type TestRawWriter =
    Box<dyn FnOnce(&mut dyn std::io::Write, &[u8]) -> std::io::Result<()> + 'static>;

#[cfg(test)]
thread_local! {
    static TEST_RAW_WRITER: std::cell::RefCell<Option<TestRawWriter>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Route the next publication on this thread through `write`, which may stage a
/// partial prefix and then fail.
#[cfg(test)]
pub(crate) fn inject_raw_writer(
    write: impl FnOnce(&mut dyn std::io::Write, &[u8]) -> std::io::Result<()> + 'static,
) {
    TEST_RAW_WRITER.with(|slot| {
        assert!(slot.borrow().is_none(), "a raw writer is already injected");
        *slot.borrow_mut() = Some(Box::new(write));
    });
}

include!("services/flathub.rs");

#[cfg(test)]
mod atomic_publication_tests;

#[cfg(test)]
mod tests;
