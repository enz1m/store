//! Bounded raster icons from the exact installed Flatpak deployment, without HTTP.

use super::{AppEntry, InstallScope};
use crate::bounded_image::{RemoteImage, RenderImage};
use rustix::fs::{Mode, OFlags};
use std::fs::File;
use std::io::Read;
use std::os::fd::AsFd;
use std::path::{Component, Path};
use std::time::{Duration, Instant};

const MAX_ICONS: usize = 128;
const MAX_BYTES: u64 = 1024 * 1024;
const BATCH_DEADLINE: Duration = Duration::from_secs(2);
const SIZES: [&str; 4] = ["64x64", "128x128", "48x48", "256x256"];

pub(super) struct LocalIcons {
    user: Option<File>,
    system: Option<File>,
    uid: u32,
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn checked(fd: &File, uid: u32, directory: bool) -> Option<()> {
    let st = rustix::fs::fstat(fd).ok()?;
    if st.st_uid != 0 && st.st_uid != uid || st.st_mode & 0o022 != 0 {
        return None;
    }
    let expected = if directory {
        rustix::fs::FileType::Directory
    } else {
        rustix::fs::FileType::RegularFile
    };
    (rustix::fs::FileType::from_raw_mode(st.st_mode) == expected).then_some(())
}

fn child(parent: &File, name: &str, uid: u32, directory: bool) -> Option<File> {
    if !safe_component(name) {
        return None;
    }
    let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    if directory {
        flags |= OFlags::DIRECTORY;
    }
    let fd = File::from(rustix::fs::openat(parent.as_fd(), name, flags, Mode::empty()).ok()?);
    checked(&fd, uid, directory)?;
    Some(fd)
}

fn root(path: &Path, uid: u32) -> Option<File> {
    if !path.is_absolute() {
        return None;
    }
    let mut fd = File::from(
        rustix::fs::open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .ok()?,
    );
    for part in path.components() {
        match part {
            Component::RootDir => (),
            Component::Normal(name) => fd = child(&fd, name.to_str()?, uid, true)?,
            _ => return None,
        }
    }
    Some(fd)
}

impl LocalIcons {
    #[cfg(not(test))]
    pub(super) fn from_env() -> Self {
        let uid = rustix::process::getuid().as_raw();
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")));
        Self {
            user: data.and_then(|p| root(&p.join("flatpak"), uid)),
            system: root(Path::new("/var/lib/flatpak"), uid),
            uid,
        }
    }

    #[cfg(test)]
    pub(super) fn from_env() -> Self {
        Self {
            user: None,
            system: None,
            uid: rustix::process::getuid().as_raw(),
        }
    }

    fn read(&self, entry: &AppEntry, is_cancelled: &dyn Fn() -> bool) -> Option<RemoteImage> {
        if !entry.installed
            || crate::flatpak_args::validate_app_id(&entry.id).is_err()
            || !safe_component(&entry.branch)
            || !safe_component(&entry.deployed_arch)
            || !matches!(entry.deployed_commit.len(), 12 | 64)
            || !entry
                .deployed_commit
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || is_cancelled()
        {
            return None;
        }
        let base = match entry.install_scope {
            InstallScope::User => self.user.as_ref()?,
            InstallScope::System => self.system.as_ref()?,
            InstallScope::Unknown | InstallScope::Named(_) => return None,
        };
        let mut deployment = base.try_clone().ok()?;
        for name in ["app", &entry.id, &entry.deployed_arch, &entry.branch] {
            deployment = child(&deployment, name, self.uid, true)?;
        }
        // Flatpak list truncates commits to 12 characters even with full-width columns.
        // Read the active link as a bounded hint; open its validated hash without following it.
        let active = rustix::fs::readlinkat(deployment.as_fd(), "active", Vec::new()).ok()?;
        let commit = active.to_str().ok()?;
        if commit.len() != 64
            || !commit
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !commit.starts_with(&entry.deployed_commit)
        {
            return None;
        }
        let branch = deployment;
        let deployment = child(&branch, commit, self.uid, true)?;
        for tree in ["export", "files"] {
            let mut directory = match child(&deployment, tree, self.uid, true) {
                Some(p) => p,
                None => continue,
            };
            let mut valid = true;
            for name in ["share", "icons", "hicolor"] {
                match child(&directory, name, self.uid, true) {
                    Some(p) => directory = p,
                    None => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                continue;
            }
            for size in SIZES {
                if is_cancelled() {
                    return None;
                }
                let Some(size_dir) = child(&directory, size, self.uid, true) else {
                    continue;
                };
                let Some(apps) = child(&size_dir, "apps", self.uid, true) else {
                    continue;
                };
                let Some(bytes) = child(&apps, &format!("{}.png", entry.id), self.uid, false)
                    .and_then(read_bounded)
                else {
                    continue;
                };
                if let Ok(image) = crate::bounded_image::decode_local_icon(&bytes, is_cancelled) {
                    return still_active(&branch, &active).then_some(image);
                }
            }
            // Many apps export only a scalable icon (Zen, for one).
            let scalable = child(&directory, "scalable", self.uid, true)
                .and_then(|scalable| child(&scalable, "apps", self.uid, true))
                .and_then(|apps| child(&apps, &format!("{}.svg", entry.id), self.uid, false))
                .and_then(read_bounded);
            if let Some(bytes) = scalable
                && let Some(image) =
                    crate::bounded_image::decode_local_svg_icon(&bytes, is_cancelled)
            {
                return still_active(&branch, &active).then_some(image);
            }
        }
        None
    }

    pub(super) fn hydrate(&self, entries: &mut [AppEntry], is_cancelled: &dyn Fn() -> bool) {
        let deadline = Instant::now() + BATCH_DEADLINE;
        let cancelled = || is_cancelled() || Instant::now() >= deadline;
        for entry in entries.iter_mut().take(MAX_ICONS) {
            if cancelled() {
                break;
            }
            if entry.icon_image.is_none() {
                entry.icon_image = self.read(entry, &cancelled).map(RenderImage::Remote);
            }
        }
    }
}

// One regular icon file, read completely and unchanged while it was read.
fn read_bounded(mut file: File) -> Option<Vec<u8>> {
    let before = file.metadata().ok()?;
    if before.len() == 0 || before.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 == before.len()
        && file.metadata().ok()?.modified().ok() == before.modified().ok())
    .then_some(bytes)
}

// The deployment the icon came from is still the active one.
fn still_active(branch: &File, active: &std::ffi::CStr) -> bool {
    rustix::fs::readlinkat(branch.as_fd(), "active", Vec::new())
        .is_ok_and(|now| now.as_c_str() == active)
}

#[cfg(test)]
mod tests;
