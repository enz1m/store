use super::*;

fn scratch_data_dir(tag: &str) -> std::path::PathBuf {
    let dir = crate::test_scratch::path(&format!("{tag}-{:?}", std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl Store {
    pub(crate) fn boot_for_test() -> (Self, ()) {
        let dir = scratch_data_dir("boot");
        Self::boot_for_test_at(dir)
    }

    pub(crate) fn boot_for_test_at(data_dir: std::path::PathBuf) -> (Self, ()) {
        let (mut s, task) = Store::boot_settled_at(data_dir, false, None);
        drop(task);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while s.read_owner.dispatcher().snapshot().live() != 0
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            s.read_owner.dispatcher().snapshot().live(),
            0,
            "cancelled boot reads must leave the test owner's pool"
        );
        s.loading_installed = false;
        s.loading_catalog = false;

        s.remotes = crate::support::default_remotes();
        s.remotes_load = crate::support::default_remotes_load();
        s.refresh_projection();
        (s, ())
    }
}
