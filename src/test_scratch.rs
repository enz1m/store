//! One scratch root per test process under `$TMPDIR`. The harness offers no
//! teardown, so each run removes the roots of earlier runs whose process is
//! gone; at most the latest run's root remains.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const PREFIX: &str = "store-tests-";

fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let temp = std::env::temp_dir();
        remove_finished(&temp);
        let root = temp.join(format!("{PREFIX}{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create test scratch root");
        root
    })
}

fn remove_finished(temp: &Path) {
    let Ok(entries) = std::fs::read_dir(temp) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|name| name.strip_prefix(PREFIX)) else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if !Path::new("/proc").join(pid.to_string()).exists() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `name` inside this process's scratch root; not created.
pub(crate) fn path(name: &str) -> PathBuf {
    root().join(name)
}
