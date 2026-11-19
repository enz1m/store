use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture {
    dir: std::path::PathBuf,
    provider: LocalIcons,
    entry: AppEntry,
    icon: std::path::PathBuf,
}
impl Fixture {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("store-local-icons-{}-{label}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let entry = AppEntry {
            id: "org.example.Private".into(),
            installed: true,
            install_scope: InstallScope::User,
            branch: "stable".into(),
            deployed_arch: "x86_64".into(),
            deployed_commit: "a".repeat(64),
            ..Default::default()
        };
        let icon = dir
            .join("app")
            .join(&entry.id)
            .join(&entry.deployed_arch)
            .join(&entry.branch)
            .join(&entry.deployed_commit)
            .join("export/share/icons/hicolor/64x64/apps/org.example.Private.png");
        std::fs::create_dir_all(icon.parent().unwrap()).unwrap();
        let branch = dir
            .join("app")
            .join(&entry.id)
            .join(&entry.deployed_arch)
            .join(&entry.branch);
        symlink(&entry.deployed_commit, branch.join("active")).unwrap();
        let provider = LocalIcons {
            user: Some(File::open(&dir).unwrap()),
            system: None,
            uid: rustix::process::getuid().as_raw(),
        };
        Self {
            dir,
            provider,
            entry,
            icon,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn raster() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&70u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&2i32.to_le_bytes());
    bytes.extend_from_slice(&2i32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    bytes.extend_from_slice(&[255; 16]);
    bytes
}

#[test]
fn installed_identity_raster_and_scope_are_required() {
    let mut f = Fixture::new("identity");
    assert!(root(Path::new("relative"), f.provider.uid).is_none());
    std::fs::write(&f.icon, raster()).unwrap();
    assert!(f.provider.read(&f.entry, &|| false).is_some());
    let saved = f.entry.clone();
    f.entry.deployed_commit.truncate(12);
    assert!(f.provider.read(&f.entry, &|| false).is_some());
    let active = f
        .dir
        .join("app")
        .join(&f.entry.id)
        .join(&f.entry.deployed_arch)
        .join(&f.entry.branch)
        .join("active");
    for target in [
        "../outside".to_owned(),
        "b".repeat(64),
        "a".repeat(65),
        "A".repeat(64),
    ] {
        std::fs::remove_file(&active).unwrap();
        symlink(target, &active).unwrap();
        assert!(f.provider.read(&f.entry, &|| false).is_none());
    }
    std::fs::remove_file(&active).unwrap();
    symlink(&saved.deployed_commit, &active).unwrap();
    f.entry = saved.clone();
    for scope in [
        InstallScope::Unknown,
        InstallScope::Named("extra".into()),
        InstallScope::System,
    ] {
        f.entry.install_scope = scope;
        assert!(f.provider.read(&f.entry, &|| false).is_none());
    }
    f.entry = saved.clone();
    f.entry.installed = false;
    assert!(f.provider.read(&f.entry, &|| false).is_none());
    f.entry = saved.clone();
    f.entry.deployed_commit = "b".repeat(64);
    assert!(f.provider.read(&f.entry, &|| false).is_none());
    f.entry = saved;
    f.entry.branch = "../stable".into();
    assert!(f.provider.read(&f.entry, &|| false).is_none());
}

#[test]
fn symlink_replacement_and_writable_ancestors_are_refused() {
    let f = Fixture::new("symlink");
    let outside = f.dir.join("outside");
    std::fs::write(&outside, raster()).unwrap();
    symlink(&outside, &f.icon).unwrap();
    assert!(f.provider.read(&f.entry, &|| false).is_none());
    std::fs::remove_file(&f.icon).unwrap();
    std::fs::write(&f.icon, raster()).unwrap();
    assert!(f.provider.read(&f.entry, &|| false).is_some());
    std::fs::set_permissions(
        f.icon.parent().unwrap(),
        std::fs::Permissions::from_mode(0o777),
    )
    .unwrap();
    assert!(f.provider.read(&f.entry, &|| false).is_none());
    std::fs::set_permissions(
        f.icon.parent().unwrap(),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let held = f.dir.with_extension("held");
    std::fs::rename(&f.dir, &held).unwrap();
    std::fs::create_dir(&f.dir).unwrap();
    assert!(f.provider.read(&f.entry, &|| false).is_some());
    std::fs::remove_dir_all(held).unwrap();
}
