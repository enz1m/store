//! Store capabilities: the data and cache tiers resolved over
//! validated roots.

use super::{
    AUTOMATIC_UPDATES_FILE, CATALOG_CACHE_SUBDIR, CATALOG_STORE_FILE, FEATURED_IMAGE_SUBDIR,
    ICON_CACHE_SUBDIR, MAX_RETAINED_CACHE_AUTHORITIES, RECENT_SEARCHES_FILE,
    SCREENSHOT_CACHE_SUBDIR, SecureDir,
};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
/// The data tier: the validated data app root held open, for the recent-search
/// authority that lives beneath it.
///
/// The recent-search file keeps the framework state authority for its atomic
/// publication and inode custody; this capability retains the parent descriptor
/// so a pathname substitution of the data root is a detectable refusal signal.
#[derive(Clone, Debug)]
pub(crate) struct DataCapability {
    pub(super) root: Arc<SecureDir>,
    pub(super) root_path: PathBuf,
}

impl DataCapability {
    /// The recent-search history file beneath the retained data root.
    #[must_use]
    pub(crate) fn recent_searches_path(&self) -> PathBuf {
        self.root_path.join(RECENT_SEARCHES_FILE)
    }

    /// Whether Flatpak apps update in the background: on unless turned off.
    #[must_use]
    pub(crate) fn automatic_updates(&self) -> bool {
        !matches!(
            self.root.read_bounded(AUTOMATIC_UPDATES_FILE, 16),
            Ok(Some(bytes)) if bytes.trim_ascii() == b"off"
        )
    }

    pub(crate) fn set_automatic_updates(&self, on: bool) -> io::Result<()> {
        self.root
            .write_atomic(AUTOMATIC_UPDATES_FILE, if on { b"on\n" } else { b"off\n" })
    }

    /// A capability over an already-trusted scratch directory, for tests.
    #[cfg(test)]
    pub(crate) fn for_test(data_dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let root = SecureDir::open_root(data_dir)?;
        Ok(Self {
            root: Arc::new(root),
            root_path: data_dir.to_path_buf(),
        })
    }
}

/// The cache tier: the validated cache app root and its descriptor-bound
/// subdirectory capabilities.
///
/// The icon, screenshot, and featured caches are read and written through their
/// held descriptors. The catalog index keeps the framework file backend for its
/// bounded atomic publication; the retained cache-root descriptor guards it as
/// a substitution signal.
#[derive(Clone, Debug)]
pub(crate) struct CacheCapability {
    root: Arc<SecureDir>,
    root_path: PathBuf,
    icons: Arc<SecureDir>,
    catalogs: Arc<SecureDir>,
    catalogs_path: PathBuf,
    screenshots: Arc<SecureDir>,
    catalog_store_path: PathBuf,
    featured_cache_path: PathBuf,
    state_authorities: Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, enzui_core::state_authority::StateAuthority>,
        >,
    >,
    latest_detail_generation: Arc<AtomicU64>,
    detail_publication_closed: Arc<std::sync::atomic::AtomicBool>,
    detail_publication: Arc<std::sync::Mutex<()>>,
}

impl CacheCapability {
    /// The icon cache capability.
    #[must_use]
    pub(crate) fn icons(&self) -> &SecureDir {
        &self.icons
    }

    /// The screenshot cache capability.
    #[must_use]
    pub(crate) fn screenshots(&self) -> &SecureDir {
        &self.screenshots
    }

    pub(crate) fn admit_detail_generation(&self, generation: u64) -> bool {
        let _publication = self
            .detail_publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if generation == 0
            || self
                .detail_publication_closed
                .load(std::sync::atomic::Ordering::Acquire)
            || generation <= self.latest_detail_generation.load(Ordering::Acquire)
        {
            return false;
        }
        self.latest_detail_generation
            .store(generation, Ordering::Release);
        true
    }

    pub(crate) fn close_detail_publication(&self) {
        let _publication = self
            .detail_publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.detail_publication_closed
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn resume_detail_publication(&self, generation: u64) -> bool {
        let _publication = self
            .detail_publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if generation == 0 || generation <= self.latest_detail_generation.load(Ordering::Acquire) {
            return false;
        }
        self.latest_detail_generation
            .store(generation, Ordering::Release);
        self.detail_publication_closed
            .store(false, std::sync::atomic::Ordering::Release);
        true
    }

    pub(crate) fn publish_detail_image(
        &self,
        name: &str,
        generation: u64,
        bytes: &[u8],
        is_cancelled: &dyn Fn() -> bool,
    ) -> io::Result<bool> {
        self.publish_detail_image_with(name, generation, bytes, is_cancelled, &|| {}, &|| {})
    }

    fn publish_detail_image_with(
        &self,
        name: &str,
        generation: u64,
        bytes: &[u8],
        is_cancelled: &dyn Fn() -> bool,
        before_temporary: &dyn Fn(),
        before_publication: &dyn Fn(),
    ) -> io::Result<bool> {
        self.screenshots
            .write_atomic_with(name, bytes, before_temporary, || {
                before_publication();
                let publication = self
                    .detail_publication
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (!is_cancelled()
                    && !self
                        .detail_publication_closed
                        .load(std::sync::atomic::Ordering::Acquire)
                    && generation == self.latest_detail_generation.load(Ordering::Acquire))
                .then_some(publication)
            })
    }

    pub(crate) fn read_legacy_catalog_cache(
        &self,
        name: &str,
        cap: u64,
    ) -> io::Result<Option<Vec<u8>>> {
        self.catalogs.read_bounded(name, cap)
    }

    pub(crate) fn read_state_cache(&self, name: &str, cap: usize) -> io::Result<Option<Vec<u8>>> {
        let mut authorities = self
            .state_authorities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !authorities.contains_key(name) && authorities.len() >= MAX_RETAINED_CACHE_AUTHORITIES {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "cache authority limit reached",
            ));
        }
        let authority = match authorities.entry(name.to_owned()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let authority =
                    enzui_core::state_authority::StateAuthority::open_or_create_tightening(
                        &self.catalogs_path.join(name),
                    )?;
                entry.insert(authority)
            }
        };
        match authority.read_bounded(cap) {
            Ok(enzui_core::state_authority::CustodySnapshot::Absent) => Ok(None),
            Ok(enzui_core::state_authority::CustodySnapshot::Current {
                bytes,
                overflow: false,
                ..
            }) => Ok(Some(bytes)),
            Ok(
                enzui_core::state_authority::CustodySnapshot::Current { overflow: true, .. }
                | enzui_core::state_authority::CustodySnapshot::Lost,
            ) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cache state is not readable",
            )),
            Err(error) => Err(io::Error::other(error)),
        }
    }

    pub(crate) fn publish_state_cache(
        &self,
        name: &str,
        generation: u64,
        bytes: &[u8],
    ) -> io::Result<bool> {
        if generation == 0
            || self
                .detail_publication_closed
                .load(std::sync::atomic::Ordering::Acquire)
            || generation != self.latest_detail_generation.load(Ordering::Acquire)
        {
            return Ok(false);
        }
        let mut authorities = self
            .state_authorities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self
            .detail_publication_closed
            .load(std::sync::atomic::Ordering::Acquire)
            || generation != self.latest_detail_generation.load(Ordering::Acquire)
        {
            return Ok(false);
        }
        if !authorities.contains_key(name) && authorities.len() >= MAX_RETAINED_CACHE_AUTHORITIES {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "cache authority limit reached",
            ));
        }
        let authority = match authorities.entry(name.to_owned()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let authority =
                    enzui_core::state_authority::StateAuthority::open_or_create_tightening(
                        &self.catalogs_path.join(name),
                    )?;
                entry.insert(authority)
            }
        };
        authority
            .publish(bytes)
            .map(|_| true)
            .map_err(io::Error::other)
    }

    /// The catalog index persistence file beneath the validated cache root.
    ///
    /// The framework file backend opens this path itself; the retained cache
    /// descriptor guards it as a substitution signal.
    #[must_use]
    pub(crate) fn catalog_store_path(&self) -> &Path {
        &self.catalog_store_path
    }

    /// The featured-image cache directory beneath the validated cache root.
    ///
    /// The framework image service opens this directory itself; the retained
    /// cache descriptor guards it as a substitution signal.
    #[must_use]
    pub(crate) fn featured_cache_path(&self) -> &Path {
        &self.featured_cache_path
    }

    /// Whether the cache root has been replaced by pathname since capture.
    #[must_use]
    pub(crate) fn substituted(&self) -> bool {
        self.root.substituted_at(&self.root_path)
    }

    /// Builds subdirectory capabilities beneath an opened cache root.
    pub(super) fn from_root(root: SecureDir, root_path: PathBuf) -> io::Result<Self> {
        let icons = Arc::new(root.child(ICON_CACHE_SUBDIR)?);
        let catalogs = Arc::new(root.child(CATALOG_CACHE_SUBDIR)?);
        let screenshots = Arc::new(root.child(SCREENSHOT_CACHE_SUBDIR)?);
        // The featured hero uses the framework image service, which opens this
        // directory by path; create it private here and retain only its path.
        root.child(FEATURED_IMAGE_SUBDIR)?;
        let catalogs_path = root_path.join(CATALOG_CACHE_SUBDIR);
        let catalog_store_path = root_path.join(CATALOG_STORE_FILE);
        let featured_cache_path = root_path.join(FEATURED_IMAGE_SUBDIR);
        Ok(Self {
            root: Arc::new(root),
            root_path,
            icons,
            catalogs,
            catalogs_path,
            screenshots,
            catalog_store_path,
            featured_cache_path,
            state_authorities: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            latest_detail_generation: Arc::new(AtomicU64::new(0)),
            detail_publication_closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            detail_publication: Arc::new(std::sync::Mutex::new(())),
        })
    }

    /// A capability over an already-trusted scratch cache directory, for tests.
    #[cfg(test)]
    pub(crate) fn for_test(cache_dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(cache_dir)?;
        let root = SecureDir::open_root(cache_dir)?;
        Self::from_root(root, cache_dir.to_path_buf())
    }
}
