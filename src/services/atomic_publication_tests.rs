//! Recent-search publication and scratch-directory ownership tests.

use super::*;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Name of the token that records which guard owns a scratch directory.
const OWNER_TOKEN: &str = ".owner-token";

/// A scratch directory this test exclusively created.
///
/// store-app has no scratch-directory dependency, so ownership is established
/// the only way a name can establish it: `create_dir` succeeding. A name that
/// already exists is skipped, never adopted and never removed. Cleanup is
/// bound to the token written at claim time, so a directory whose token has
/// changed hands is left alone.
struct OwnedScratch {
    path: PathBuf,
    token: String,
}

impl OwnedScratch {
    fn new(label: &str) -> Self {
        Self::claim(&scratch_root(), candidate_names(label))
    }

    fn claim(root: &Path, candidates: impl IntoIterator<Item = String>) -> Self {
        std::fs::create_dir_all(root).expect("create the scratch root");
        for leaf in candidates {
            let path = root.join(leaf);
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    let token = unique_suffix();
                    std::fs::write(path.join(OWNER_TOKEN), token.as_bytes())
                        .expect("write the ownership token");
                    return Self { path, token };
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot claim {}: {error}", path.display()),
            }
        }
        panic!("every candidate scratch name was already in use");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for OwnedScratch {
    fn drop(&mut self) {
        let held = std::fs::read(self.path.join(OWNER_TOKEN)).ok();
        if held.as_deref() == Some(self.token.as_bytes()) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before the epoch")
        .as_nanos();
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

fn candidate_names(label: &str) -> impl Iterator<Item = String> {
    let label = label.to_owned();
    (0..64).map(move |_| format!("{label}-{}", unique_suffix()))
}

// The scratch root lives in the active build's target tree, never in the
// system temp directory a stranger can create names in first.
fn scratch_root() -> PathBuf {
    let executable = std::env::current_exe().expect("locate the test executable");
    let deps = executable.parent().expect("test executable has a parent");
    assert_eq!(
        deps.file_name().and_then(|name| name.to_str()),
        Some("deps"),
        "unexpected test executable location: {}",
        executable.display()
    );
    deps.parent()
        .expect("deps has a parent profile directory")
        .join("store-app-publication-scratch")
}

fn inode(path: &Path) -> u64 {
    std::fs::metadata(path).expect("publication metadata").ino()
}

// The staged prefix must reach a byte differing from the seeded document, so
// an in-place overwrite cannot leave the old file intact.
fn first_differing_index(old: &[u8], new: &[u8]) -> usize {
    let index = old
        .iter()
        .zip(new)
        .position(|(a, b)| a != b)
        .expect("the two fixtures must differ inside their common prefix");
    assert_ne!(old[index], new[index]);
    index
}

// Everything in `dir` except the published file and the scratch guard's own
// ownership token.
fn entries_besides(dir: &Path, keep: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .expect("read publication directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path != keep && path.file_name() != Some(OWNER_TOKEN.as_ref()))
        .collect()
}

fn arm_interrupted_prefix(old_bytes: &[u8], new_bytes: &[u8]) -> usize {
    let index = first_differing_index(old_bytes, new_bytes);
    let old_byte = old_bytes[index];
    let new_byte = new_bytes[index];
    inject_raw_writer(move |staged, bytes| {
        assert_eq!(bytes[index], new_byte);
        assert_ne!(bytes[index], old_byte);
        staged.write_all(&bytes[..=index])?;
        Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "injected after the staged prefix",
        ))
    });
    index
}

#[test]
fn recent_search_publication_survives_an_interrupted_staged_write() {
    let scratch = OwnedScratch::new("recent-searches");
    let dir = scratch.path();
    let path = recent_searches_path_at(dir);

    let old = vec!["alpha".to_owned(), "bravo".to_owned()];
    save_recent_searches_at(&path, &old).expect("seed the previous list");
    let old_bytes = std::fs::read(&path).expect("seeded bytes");
    let seeded_inode = inode(&path);

    let replacement = vec!["charlie".to_owned(), "delta".to_owned()];
    let replacement_bytes = replacement
        .iter()
        .map(|q| format!("{q}\n"))
        .collect::<String>()
        .into_bytes();
    let _ = arm_interrupted_prefix(&old_bytes, &replacement_bytes);

    let error = save_recent_searches_at(&path, &replacement)
        .expect_err("the interrupted write must be reported");
    assert!(error.starts_with("write:"), "{error}");
    assert_eq!(std::fs::read(&path).expect("old bytes"), old_bytes);
    assert_eq!(
        inode(&path),
        seeded_inode,
        "the old file was replaced anyway"
    );
    assert_eq!(load_recent_searches_at(&path), old);
    assert!(
        entries_besides(dir, &path).is_empty(),
        "the interrupted write left staging residue"
    );

    save_recent_searches_at(&path, &replacement).expect("the retry publishes");
    let published = std::fs::read(&path).expect("published bytes");
    assert_eq!(published, replacement_bytes);
    assert_ne!(
        inode(&path),
        seeded_inode,
        "publication must install a new inode rather than rewrite the old one"
    );
    assert_eq!(load_recent_searches_at(&path), replacement);
    assert!(!published.starts_with(b"enzui-iced "));
}

// Cleanup is bound to the token, not to the path: once the token inside is
// no longer this guard's, dropping it removes nothing.
#[test]
fn the_guard_removes_only_what_its_token_still_owns() {
    let outer = OwnedScratch::new("token-handover");
    let scratch = OwnedScratch::claim(&outer.path().join("names"), ["handed-over".to_owned()]);
    let path = scratch.path().to_path_buf();
    let kept = path.join("kept");
    std::fs::write(&kept, b"another owner's file").expect("seed the file");
    std::fs::write(path.join(OWNER_TOKEN), b"a different owner").expect("hand the token over");

    drop(scratch);

    assert!(
        path.exists(),
        "cleanup removed a directory it no longer owns"
    );
    assert_eq!(
        std::fs::read(&kept).expect("kept bytes"),
        b"another owner's file"
    );
}
