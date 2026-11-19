//! The store's durable-root authority as retained directory descriptors.
//!
//! Resolution adopts the framework [`AppDataResolver`], which ignores a
//! relative root and creates an owner-checked private leaf, so no route falls
//! back to a shared `/tmp` root or a relative placeholder. Each validated app
//! root is then held open as a [`SecureDir`] descriptor. Every later cache or
//! state operation is performed relative to that descriptor with the `openat`
//! family, so replacing a validated directory by pathname with another
//! same-user private directory cannot redirect an operation: the descriptor
//! addresses the original inode. A pathname re-check is only a substitution
//! signal, never the authority for an operation.
//!
//! The data and cache tiers resolve independently. A missing or refused cache
//! does not disable data-backed history, and a missing or refused data root
//! does not disable network operation that can run with no durable cache. The
//! captured capabilities are owned by the running `Store` and cloned into the
//! exact task closures that need them; nothing reads a process-global locator.

use enzui_core::app_data::{AppDataDir, AppDataError, AppDataResolver, AppId};
use enzui_core::persist::PersistenceKind;
use rustix::fs::{AtFlags, Mode, OFlags};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// This application's data-directory segment.
const STORE_APP_ID: &str = "store-app";

/// The recent-search history file beneath the data root.
const RECENT_SEARCHES_FILE: &str = "recent-searches.txt";

/// Whether Flatpak apps update in the background, beneath the data root.
/// `packaging/flatpak-auto-update` reads it by this name: `off` turns the
/// daily updates off.
const AUTOMATIC_UPDATES_FILE: &str = "automatic-updates";

/// The icon cache subdirectory beneath the cache root.
const ICON_CACHE_SUBDIR: &str = "icons-v2";

/// The catalog cache subdirectory beneath the cache root.
const CATALOG_CACHE_SUBDIR: &str = "catalogs";

/// The screenshot cache subdirectory beneath the cache root.
const SCREENSHOT_CACHE_SUBDIR: &str = "screenshots-v2";

/// The catalog index persistence file beneath the cache root.
const CATALOG_STORE_FILE: &str = "catalog";

/// The featured-image cache subdirectory beneath the cache root.
const FEATURED_IMAGE_SUBDIR: &str = "featured-images";

/// The private mode for a store-owned cache subdirectory.
const PRIVATE_DIR_MODE: u32 = 0o700;

/// The private mode for a store-owned cache file.
const PRIVATE_FILE_MODE: u32 = 0o600;

const MAX_RETAINED_CACHE_AUTHORITIES: usize = 64;

/// The device and inode of a validated directory, captured when it was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}

impl Identity {
    fn of(fd: BorrowedFd<'_>) -> io::Result<Self> {
        let stat = rustix::fs::fstat(fd).map_err(io::Error::from)?;
        Ok(Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        })
    }

    fn at(path: &Path) -> io::Result<Self> {
        let stat = rustix::fs::statat(rustix::fs::CWD, path, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(io::Error::from)?;
        Ok(Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        })
    }

    fn relative_to(dir: BorrowedFd<'_>, name: &str) -> io::Result<Self> {
        let stat =
            rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
        Ok(Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        })
    }
}

/// An owned descriptor to a validated directory. Every file operation is issued
/// relative to this descriptor, so a pathname substitution cannot redirect it.
#[derive(Debug)]
pub(crate) struct SecureDir {
    fd: OwnedFd,
    identity: Identity,
    temporary: std::sync::Mutex<()>,
}

impl SecureDir {
    fn entry_name_is_valid(name: &str) -> bool {
        let mut components = Path::new(name).components();
        matches!(components.next(), Some(std::path::Component::Normal(_)))
            && components.next().is_none()
    }

    // Opens an existing directory by absolute path as a descriptor capability,
    // refusing a symlink at the final component.
    fn open_root(path: &Path) -> io::Result<Self> {
        let fd = rustix::fs::openat(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?;
        let identity = Identity::of(fd.as_fd())?;
        Ok(Self {
            fd,
            identity,
            temporary: std::sync::Mutex::new(()),
        })
    }

    // Opens a child directory relative to this one, creating it private if
    // absent, making an open one of ours private, and refusing a symlink or
    // another user's child.
    fn child(&self, name: &str) -> io::Result<Self> {
        match rustix::fs::mkdirat(&self.fd, name, Mode::from_raw_mode(PRIVATE_DIR_MODE)) {
            Ok(()) => {}
            Err(error) if error == rustix::io::Errno::EXIST => {}
            Err(error) => return Err(io::Error::from(error)),
        }
        let fd = rustix::fs::openat(
            &self.fd,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?;
        let mut stat = rustix::fs::fstat(&fd).map_err(io::Error::from)?;
        if stat.st_uid != rustix::process::geteuid().as_raw() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cache directory is not owned by this user",
            ));
        }
        if stat.st_mode & 0o077 != 0 {
            // Earlier builds created these with the umask's 0755. Ours, and
            // held open without following links, so make it private.
            rustix::fs::fchmod(&fd, Mode::from_raw_mode(PRIVATE_DIR_MODE))
                .map_err(io::Error::from)?;
            stat = rustix::fs::fstat(&fd).map_err(io::Error::from)?;
            if stat.st_mode & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "cache directory is not owner-private",
                ));
            }
        }
        let identity = Identity::of(fd.as_fd())?;
        Ok(Self {
            fd,
            identity,
            temporary: std::sync::Mutex::new(()),
        })
    }

    fn private_regular_identity(fd: BorrowedFd<'_>) -> io::Result<Identity> {
        let stat = rustix::fs::fstat(fd).map_err(io::Error::from)?;
        if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cache entry is not a regular file",
            ));
        }
        if stat.st_uid != rustix::process::geteuid().as_raw()
            || stat.st_mode & 0o077 != 0
            || stat.st_nlink != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cache entry is not owner-private",
            ));
        }
        Identity::of(fd)
    }

    /// Opens a singly linked, owner-private regular file without following links.
    /// `None` means the entry is absent.
    pub(crate) fn open_read(&self, name: &str) -> io::Result<Option<File>> {
        if !Self::entry_name_is_valid(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache entry name is not a single relative component",
            ));
        }
        let fd = match rustix::fs::openat(
            &self.fd,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
            Err(error) => return Err(io::Error::from(error)),
        };
        Self::private_regular_identity(fd.as_fd())?;
        Ok(Some(File::from(fd)))
    }

    /// Reads a file relative to this directory, admitting up to `limit` bytes
    /// and refusing one more. `None` when the entry is absent.
    pub(crate) fn read_bounded(&self, name: &str, limit: u64) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = self.open_read(name)? else {
            return Ok(None);
        };
        let mut buf = Vec::new();
        file.take(limit.saturating_add(1)).read_to_end(&mut buf)?;
        if buf.len() as u64 > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cache entry exceeds its byte ceiling",
            ));
        }
        Ok(Some(buf))
    }

    pub(crate) fn write_atomic(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        if self.write_atomic_with(name, bytes, &|| {}, || Some(()))? {
            Ok(())
        } else {
            Err(io::Error::other("cache publication refused"))
        }
    }

    pub(crate) fn write_atomic_if(
        &self,
        name: &str,
        bytes: &[u8],
        may_publish: impl FnOnce() -> bool,
    ) -> io::Result<bool> {
        self.write_atomic_with(name, bytes, &|| {}, || may_publish().then_some(()))
    }

    fn write_atomic_with<T>(
        &self,
        name: &str,
        bytes: &[u8],
        before_temporary: &dyn Fn(),
        prepare_publication: impl FnOnce() -> Option<T>,
    ) -> io::Result<bool> {
        if !Self::entry_name_is_valid(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache entry name is not a single relative component",
            ));
        }
        before_temporary();
        let _temporary = self
            .temporary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = format!(".{name}.tmp");
        self.clear_interrupted_temporary(&tmp)?;
        let fd = rustix::fs::openat(
            &self.fd,
            tmp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(PRIVATE_FILE_MODE),
        )
        .map_err(io::Error::from)?;
        let tmp_identity = Self::private_regular_identity(fd.as_fd())?;
        let mut file = File::from(fd);
        file.write_all(bytes)?;
        file.sync_all()?;

        if Identity::relative_to(self.fd.as_fd(), &tmp)? != tmp_identity {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cache temporary identity changed",
            ));
        }
        self.verify_replaceable(name)?;
        let Some(permit) = prepare_publication() else {
            return Ok(false);
        };
        let outcome =
            rustix::fs::renameat(&self.fd, tmp.as_str(), &self.fd, name).map_err(io::Error::from);
        drop(permit);
        outcome.map(|()| true)
    }

    fn clear_interrupted_temporary(&self, name: &str) -> io::Result<()> {
        let observed = match rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(()),
            Err(error) => return Err(io::Error::from(error)),
        };
        if rustix::fs::FileType::from_raw_mode(observed.st_mode)
            != rustix::fs::FileType::RegularFile
            || observed.st_uid != rustix::process::geteuid().as_raw()
            || observed.st_mode & 0o077 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cache temporary is not an owner-private regular file",
            ));
        }
        let observed = Identity {
            device: observed.st_dev,
            inode: observed.st_ino,
        };
        if Identity::relative_to(self.fd.as_fd(), name)? != observed {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cache temporary identity changed",
            ));
        }
        rustix::fs::unlinkat(&self.fd, name, AtFlags::empty()).map_err(io::Error::from)
    }

    fn verify_replaceable(&self, name: &str) -> io::Result<()> {
        let fd = match rustix::fs::openat(
            &self.fd,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(()),
            Err(error) => return Err(io::Error::from(error)),
        };
        Self::private_regular_identity(fd.as_fd()).map(|_| ())
    }

    // Whether the given pathname now resolves to a different inode than the one
    // this descriptor was opened at. A substitution signal, not the operation
    // authority.
    #[must_use]
    fn substituted_at(&self, path: &Path) -> bool {
        Identity::at(path).map_or(true, |now| now != self.identity)
    }
}

/// The independently resolved data and cache tiers.
///
/// Each tier is available or carries a path-free reason of its own, so one
/// tier's failure never disables the other.
#[derive(Debug)]
pub(crate) struct StoreCapabilities {
    data: Result<DataCapability, RootsUnavailable>,
    cache: Result<CacheCapability, RootsUnavailable>,
}

impl StoreCapabilities {
    /// The data capability, when the data tier resolved.
    #[must_use]
    pub(crate) fn data(&self) -> Option<&DataCapability> {
        self.data.as_ref().ok()
    }

    /// The cache capability, when the cache tier resolved.
    #[must_use]
    pub(crate) fn cache(&self) -> Option<&CacheCapability> {
        self.cache.as_ref().ok()
    }

    /// The path-free reason the data tier is unavailable, if it is.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn data_unavailable(&self) -> Option<RootsUnavailable> {
        self.data.as_ref().err().copied()
    }

    /// The path-free reason the cache tier is unavailable, if it is.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn cache_unavailable(&self) -> Option<RootsUnavailable> {
        self.cache.as_ref().err().copied()
    }

    /// A capabilities bundle over already-trusted scratch directories, for
    /// tests that inject a private data root and a `cache` child of it.
    #[cfg(test)]
    pub(crate) fn for_test(data_dir: &Path) -> Self {
        Self::for_test_tiers(data_dir, true, true)
    }

    /// A partial capabilities bundle, for a control that a store with only one
    /// tier available gates the right operations.
    #[cfg(test)]
    pub(crate) fn for_test_tiers(data_dir: &Path, data_ok: bool, cache_ok: bool) -> Self {
        let cache_dir = data_dir.join("cache");
        Self {
            data: if data_ok {
                DataCapability::for_test(data_dir).map_err(|_| RootsUnavailable::DataRootRefused)
            } else {
                Err(RootsUnavailable::DataRootRefused)
            },
            cache: if cache_ok {
                CacheCapability::for_test(&cache_dir)
                    .map_err(|_| RootsUnavailable::CacheRootRefused)
            } else {
                Err(RootsUnavailable::CacheRootRefused)
            },
        }
    }
}

/// Injectable XDG and home roots for the store's data and cache directories.
///
/// The two kind roots and `$HOME` are captured as data; resolution reads no
/// environment. A test constructs one with [`StoreRootResolver::new`]; the
/// running application captures the process environment once with
/// [`StoreRootResolver::from_env`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoreRootResolver {
    data_home: Option<PathBuf>,
    cache_home: Option<PathBuf>,
    home: Option<PathBuf>,
}

impl StoreRootResolver {
    /// Builds a resolver from explicit roots; `None` means "not set".
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(
        data_home: Option<impl Into<PathBuf>>,
        cache_home: Option<impl Into<PathBuf>>,
        home: Option<impl Into<PathBuf>>,
    ) -> Self {
        Self {
            data_home: data_home.map(Into::into),
            cache_home: cache_home.map(Into::into),
            home: home.map(Into::into),
        }
    }

    /// Snapshots `$XDG_DATA_HOME`, `$XDG_CACHE_HOME`, and `$HOME` from the
    /// process environment.
    #[cfg(not(test))]
    #[must_use]
    pub(crate) fn from_env() -> Self {
        fn non_empty(key: &str) -> Option<PathBuf> {
            let value = std::env::var_os(key)?;
            if value.is_empty() {
                None
            } else {
                Some(PathBuf::from(value))
            }
        }
        Self {
            data_home: non_empty("XDG_DATA_HOME"),
            cache_home: non_empty("XDG_CACHE_HOME"),
            home: non_empty("HOME"),
        }
    }

    /// In test builds, resolves under a per-process temp root instead of the
    /// developer's real `$HOME`, so a `Store::boot()` under test never creates
    /// directories in the real data or cache home.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn from_env() -> Self {
        let base = crate::test_scratch::path("boot-env");
        Self {
            data_home: Some(base.join("data")),
            cache_home: Some(base.join("cache")),
            home: None,
        }
    }

    /// Resolves both tiers independently, holding each validated root open.
    #[must_use]
    pub(crate) fn resolve(&self) -> StoreCapabilities {
        StoreCapabilities {
            data: self.resolve_data(),
            cache: self.resolve_cache(),
        }
    }

    // Resolves, creates, and opens the validated private data root. A root of
    // ours an earlier build left open (0755) is made private, not refused.
    fn resolve_data(&self) -> Result<DataCapability, RootsUnavailable> {
        let app = store_app_id();
        let data = AppDataResolver::new(self.data_home.clone(), self.home.clone())
            .ensure_tightening(&app)
            .map_err(RootsUnavailable::from_data)?;
        let root_path = data.path().to_path_buf();
        let root = open_validated_root(&data).map_err(|_| RootsUnavailable::DataRootRefused)?;
        Ok(DataCapability {
            root: Arc::new(root),
            root_path,
        })
    }

    // Resolves, creates, and opens the validated private cache root and its
    // descriptor-bound subdirectories, making an open (0755) root of ours
    // private as the data root does.
    fn resolve_cache(&self) -> Result<CacheCapability, RootsUnavailable> {
        let app = store_app_id();
        let cache_base = self.cache_base()?;
        let cache = AppDataResolver::new(Some(cache_base), None::<PathBuf>)
            .ensure_tightening(&app)
            .map_err(RootsUnavailable::from_cache)?;
        let root_path = cache.path().to_path_buf();
        let root = open_validated_root(&cache).map_err(|_| RootsUnavailable::CacheRootRefused)?;
        CacheCapability::from_root(root, root_path).map_err(|_| RootsUnavailable::CacheRootRefused)
    }

    // The absolute cache base directory, ignoring a relative root exactly as
    // the data resolver does.
    fn cache_base(&self) -> Result<PathBuf, RootsUnavailable> {
        if let Some(dir) = self.cache_home.as_ref().filter(|p| p.is_absolute()) {
            return Ok(dir.clone());
        }
        if let Some(home) = self.home.as_ref().filter(|p| p.is_absolute()) {
            return Ok(home.join(PersistenceKind::Cache.fallback_relative()));
        }
        Err(RootsUnavailable::NoCacheHome)
    }
}

// Opens a validated app root as a descriptor, refusing a symlink final
// component even though the resolver already owner-checked the directory.
fn open_validated_root(dir: &AppDataDir) -> io::Result<SecureDir> {
    SecureDir::open_root(dir.path())
}

/// A path-free reason a tier's durable root could not be established.
///
/// The message names no path, so an unavailable notice never leaks a resolved
/// or foreign location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootsUnavailable {
    /// No usable absolute data root: `$XDG_DATA_HOME` and `$HOME` are unset or
    /// relative.
    NoDataHome,
    /// No usable absolute cache root: `$XDG_CACHE_HOME` and `$HOME` are unset or
    /// relative.
    NoCacheHome,
    /// The data app root exists but is a symlink, a non-directory, a
    /// foreign-owned directory, or can't be made private.
    DataRootRefused,
    /// The cache app root exists but is a symlink, a non-directory, a
    /// foreign-owned directory, or can't be made private.
    CacheRootRefused,
}

impl RootsUnavailable {
    // Classifies a data-root failure, dropping the failing pathname.
    fn from_data(error: AppDataError) -> Self {
        match error {
            AppDataError::NoDataHome { .. } => Self::NoDataHome,
            AppDataError::InvalidAppId { .. } | AppDataError::Create { .. } => {
                Self::DataRootRefused
            }
        }
    }

    // Classifies a cache-root failure, dropping the failing pathname.
    fn from_cache(error: AppDataError) -> Self {
        match error {
            AppDataError::NoDataHome { .. } => Self::NoCacheHome,
            AppDataError::InvalidAppId { .. } | AppDataError::Create { .. } => {
                Self::CacheRootRefused
            }
        }
    }
}

// The validated single-segment application id. `store-app` is a compile-time
// constant that satisfies the id policy, so construction cannot fail.
fn store_app_id() -> AppId {
    AppId::new(STORE_APP_ID).expect("store-app is a valid application id")
}

#[path = "roots/capabilities.rs"]
mod capabilities;

pub(crate) use capabilities::{CacheCapability, DataCapability};

#[cfg(test)]
mod tests {
    use super::*;

    /// The two roots this authority governs. A refusal control iterates the
    /// pair so both roots are exercised, not one hand-picked root.
    #[derive(Clone, Copy, Debug)]
    enum Root {
        Data,
        Cache,
    }

    const ROOTS: [Root; 2] = [Root::Data, Root::Cache];

    fn sandbox(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "store-3044-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the roots sandbox");
        dir
    }

    // A resolver whose data and cache bases are distinct absolute subdirectories
    // of one sandbox, with the sabotaged root's app directory pre-seeded.
    fn resolver_with(
        dir: &Path,
        sabotage: Option<Root>,
        seed: impl Fn(&Path),
    ) -> StoreRootResolver {
        let data_home = dir.join("data");
        let cache_home = dir.join("cache");
        std::fs::create_dir_all(&data_home).expect("data base");
        std::fs::create_dir_all(&cache_home).expect("cache base");
        if let Some(root) = sabotage {
            let base = match root {
                Root::Data => &data_home,
                Root::Cache => &cache_home,
            };
            seed(&base.join(STORE_APP_ID));
        }
        StoreRootResolver::new(Some(data_home), Some(cache_home), None::<PathBuf>)
    }

    fn expected_refusal(root: Root) -> RootsUnavailable {
        match root {
            Root::Data => RootsUnavailable::DataRootRefused,
            Root::Cache => RootsUnavailable::CacheRootRefused,
        }
    }

    #[test]
    fn missing_and_relative_homes_refuse_every_tier_and_touch_no_filesystem() {
        let relative = PathBuf::from("store-3044-relative-should-not-exist");
        assert!(!relative.exists());

        let no_data = StoreRootResolver::new(None::<PathBuf>, None::<PathBuf>, None::<PathBuf>);
        let caps = no_data.resolve();
        assert_eq!(caps.data_unavailable(), Some(RootsUnavailable::NoDataHome));
        assert_eq!(
            caps.cache_unavailable(),
            Some(RootsUnavailable::NoCacheHome)
        );

        let data_base =
            std::env::temp_dir().join(format!("store-3044-data-{}", std::process::id()));
        let no_cache = StoreRootResolver::new(
            Some(data_base.clone()),
            Some(relative.clone()),
            None::<PathBuf>,
        );
        let caps = no_cache.resolve();
        assert!(
            caps.data().is_some(),
            "an absolute data base still resolves"
        );
        assert_eq!(
            caps.cache_unavailable(),
            Some(RootsUnavailable::NoCacheHome)
        );
        assert!(
            !relative.exists(),
            "a refused relative root creates nothing under the working directory"
        );

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn a_symlink_app_root_is_refused_only_for_its_own_tier() {
        let mut checked = 0;
        for root in ROOTS {
            let dir = sandbox(&format!("symlink-{root:?}"));
            let target = dir.join("elsewhere");
            std::fs::create_dir_all(&target).expect("symlink target");
            let resolver = resolver_with(&dir, Some(root), |app_root| {
                std::os::unix::fs::symlink(&target, app_root).expect("plant the symlink");
            });
            let caps = resolver.resolve();
            match root {
                Root::Data => {
                    assert_eq!(caps.data_unavailable(), Some(expected_refusal(root)));
                    assert!(caps.cache().is_some(), "the cache tier still resolves");
                }
                Root::Cache => {
                    assert_eq!(caps.cache_unavailable(), Some(expected_refusal(root)));
                    assert!(caps.data().is_some(), "the data tier still resolves");
                }
            }
            checked += 1;
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert_eq!(checked, ROOTS.len());
    }

    #[test]
    fn a_same_user_substitution_never_redirects_a_held_descriptor_for_either_root() {
        use std::os::unix::fs::PermissionsExt;
        // A data-style root and a cache-style root, each opened as a descriptor,
        // then replaced by pathname with a fresh same-user 0700 directory.
        let mut checked = 0;
        for root in ROOTS {
            let dir = sandbox(&format!("swap-{root:?}"));
            let live = dir.join("live");
            std::fs::create_dir_all(&live).expect("live root");
            let held = SecureDir::open_root(&live).expect("open the validated root");
            held.write_atomic("marker.bin", b"ORIGINAL")
                .expect("write through the descriptor");

            std::fs::rename(&live, dir.join("stash")).expect("move the real dir away");
            std::fs::create_dir(&live).expect("plant a same-user replacement");
            std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o700))
                .expect("0700 replacement");
            std::fs::write(live.join("marker.bin"), b"DECOY").expect("decoy bytes");

            let read = held
                .read_bounded("marker.bin", 64)
                .expect("read through the descriptor")
                .expect("the original entry is present");
            assert_eq!(
                read, b"ORIGINAL",
                "the held descriptor addresses the original inode, not the replacement"
            );
            assert!(
                held.substituted_at(&live),
                "the pathname substitution is detected as a refusal signal"
            );
            assert_eq!(
                std::fs::read(live.join("marker.bin")).expect("read the decoy"),
                b"DECOY",
                "the foreign replacement bytes are left untouched"
            );

            checked += 1;
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert_eq!(checked, ROOTS.len());
    }
}
