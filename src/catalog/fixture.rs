//! Capture/offline fixture catalogue: a deterministic network-free source
//! larger than its rails, for captures and tests.

use crate::services::AppEntry;

use super::{CatalogIndex, CatalogPage, CatalogSource, RailKind, ingest};

/// A deterministic, network-free catalogue.
///
/// Used by `ENZUI_FIXTURE=1` captures and by tests. Its universe is
/// deliberately **larger than its rails** — [`crate::services::mock_catalog`]
/// (12 apps, all on rails) plus [`crate::services::mock_catalog_tail`]
/// (catalogue-only apps that appear on no rail), which is the shape the defect
/// is about: an app nobody featured must still be findable.
pub(crate) struct FixtureCatalogSource {
    /// Requests this source was asked for, so a fixture-driven test can do
    /// request accounting without a network.
    calls: std::sync::atomic::AtomicU32,
}

impl Default for FixtureCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FixtureCatalogSource {
    pub(crate) fn new() -> Self {
        Self {
            calls: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// The whole fixture universe: the twelve rail apps plus the
    /// catalogue-only tail, **install-ranked descending** exactly as
    /// `/collection/popular` returns the real catalogue.
    ///
    /// The ranking is load-bearing for the fixture's meaning: the popular rail
    /// is derived from the catalogue's own head (see [`ingest`]), so a fixture
    /// served in declaration order would put arbitrary apps on the rail. Ranked,
    /// `org.mozilla.thunderbird` lands at position **24** — the first app the
    /// old `.take(24)` cut off, which is why the gates search for it by name.
    pub(crate) fn universe() -> Vec<AppEntry> {
        let mut all = crate::services::mock_catalog();
        all.extend(crate::services::mock_catalog_tail());
        all.sort_by(|a, b| b.downloads.cmp(&a.downloads).then_with(|| a.id.cmp(&b.id)));
        all
    }
}

impl CatalogSource for FixtureCatalogSource {
    fn fetch_catalog_page(&self, page: u32, _per_page: u32) -> Result<CatalogPage, String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let entries = Self::universe();
        let total = entries.len() as u32;
        Ok(CatalogPage {
            entries,
            received_hits: total,
            page,
            total_pages: 1,
            total_hits: total,
        })
    }

    fn fetch_rail(&self, kind: RailKind) -> Result<Vec<AppEntry>, String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let rails = crate::services::fixture_catalog();
        Ok(match kind {
            RailKind::Popular => rails.popular,
            RailKind::Trending => rails.trending,
            RailKind::RecentlyAdded => rails.new,
            RailKind::RecentlyUpdated => rails.updated,
        })
    }
}

/// The fixture index: ingested from [`FixtureCatalogSource`], no network, no
/// disk, no clock (`fetched_at` is a fixed stamp so captures are byte-stable).
pub(crate) fn fixture_index() -> CatalogIndex {
    // A fixed, obviously-synthetic instant: 2026-01-01T00:00:00Z. A real clock
    // here would make the stale banner's copy depend on when the capture ran.
    const FIXTURE_STAMP: u64 = 1_767_225_600;
    ingest(&FixtureCatalogSource::new(), FIXTURE_STAMP)
        .map(|r| r.index)
        .unwrap_or_default()
}
