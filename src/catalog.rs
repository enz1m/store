//! Bulk catalogue ingestion, indexed app lookup and bounded persistent caching.
//!
//! [`CatalogIndex`] owns unique app entries; ranking rails reference their IDs.
//! Search and categories use the complete index rather than editorial rails.
//! Ingestion enumerates paginated popular results and separately fetches the
//! remaining ranking rails; request count scales with pages rather than apps.
//! Cache reads and writes run outside the UI reducer and obey the complete-frame
//! ceiling in [`CATALOG_FRAME_MAX_BYTES`]. Failed refreshes preserve known cached
//! data with explicit freshness; an absent catalogue produces an actionable error.

use crate::services::AppEntry;
use std::collections::HashMap;

#[path = "catalog/fixture.rs"]
mod fixture;

pub(crate) use fixture::fixture_index;

/// Hits requested per catalogue page.
///
/// Above the catalogue's total size on purpose: Flathub returns the whole
/// catalogue in one response when `per_page` exceeds `totalHits` (measured
/// 2026-08-17: 3299 hits, `totalPages: 1`). [`ingest`] still follows
/// `totalPages`, so a catalogue that outgrows this constant costs one extra
/// request per page — never one per app.
pub(crate) const CATALOG_PAGE_SIZE: u32 = 5_000;

/// Hard ceiling on catalogue pages fetched in one ingest.
///
/// A bound the endpoint cannot talk us out of: a wrong `totalPages` (or a
/// hostile one) can cost at most this many requests.
pub(crate) const MAX_CATALOG_PAGES: u32 = 8;

/// How many apps a ranking rail contributes to Discover.
///
/// This is a *presentation* cap on the ordering rails, not the size of the
/// searchable universe — that is [`CatalogIndex::len`]. Sixty-odd rail apps
/// used to BE the catalogue; now they only order Discover.
pub(crate) const RAIL_CAP: usize = 24;

/// How long a cached catalogue is considered fresh (24 h).
pub(crate) const CATALOG_TTL_SECS: u64 = 24 * 3600;

/// `format_id` of the persisted catalogue cache.
pub(crate) const CACHE_FORMAT_ID: &str = "store-app-catalog";

/// Schema version of the persisted catalogue cache.
///
/// Icon hydration does not change the serialized schema: decoded renderer
/// handles are deliberately skipped. Keep version 1 readable and hydrate its
/// currently visible entries after load instead of turning every existing
/// cache into a write-protected version mismatch.
pub(crate) const CACHE_VERSION: u32 = 1;

/// The production complete-frame ceiling passed to the cache's [`FileBackend`].
///
/// The framework derives the framing from this cache's [`CACHE_FORMAT_ID`] and
/// [`CACHE_VERSION`] and the network body ceiling, so the allowance tracks the
/// header the atomic loader actually writes rather than a hand-maintained
/// literal. It equals [`CATALOG_FRAME_MAX_BYTES`], the documented
/// body-plus-header value the persistence gate cross-checks this derivation
/// against.
pub(crate) const CATALOG_FRAME_LIMIT: enzui_core::persist::RawByteLimit =
    match enzui_core::persist::RawByteLimit::for_body(
        CACHE_FORMAT_ID,
        CACHE_VERSION,
        crate::services::CATALOG_BODY_MAX_BYTES as u64,
    ) {
        Ok(limit) => limit,
        Err(_) => panic!("catalogue cache frame ceiling exceeds the framework ceiling"),
    };

/// Wall-clock seconds since the Unix epoch, or 0 if the clock is before it.
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------
// Rails
// ---------------------------------------------------------------------------

/// One of Flathub's four editorial collections, kept as a **ranking rail** for
/// Discover rather than as a source of apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RailKind {
    Popular,
    Trending,
    RecentlyAdded,
    RecentlyUpdated,
}

impl RailKind {
    /// All four, in Discover's declaration order.
    pub(crate) const ALL: [RailKind; 4] = [
        RailKind::Popular,
        RailKind::Trending,
        RailKind::RecentlyAdded,
        RailKind::RecentlyUpdated,
    ];

    /// The Flathub `collection/{kind}` path segment.
    pub(crate) fn endpoint(self) -> &'static str {
        match self {
            RailKind::Popular => "popular",
            RailKind::Trending => "trending",
            RailKind::RecentlyAdded => "recently-added",
            RailKind::RecentlyUpdated => "recently-updated",
        }
    }

    /// The key this rail is stored under in the cache.
    pub(crate) fn key(self) -> &'static str {
        self.endpoint()
    }

    /// The rails that need their own HTTP request.
    ///
    /// [`RailKind::Popular`] is absent: page 1 of the catalogue is already the
    /// install-ranked order, so asking for it again would be a fifth request
    /// for data we hold.
    pub(crate) const FETCHED: [RailKind; 3] = [
        RailKind::Trending,
        RailKind::RecentlyAdded,
        RailKind::RecentlyUpdated,
    ];
}

// ---------------------------------------------------------------------------
// The injectable seam
// ---------------------------------------------------------------------------

/// One page of the catalogue as the collection endpoint returns it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CatalogPage {
    pub entries: Vec<AppEntry>,
    /// Number of wire hits before invalid application IDs were discarded.
    pub received_hits: u32,
    /// `page` echoed by the endpoint (1-based).
    pub page: u32,
    /// `totalPages` — how many more requests the whole catalogue costs.
    pub total_pages: u32,
    /// `totalHits` — the catalogue size the server reports.
    pub total_hits: u32,
}

/// Bulk catalogue transport; pages are 1-based and request accounting excludes per-app enrichment.
pub(crate) trait CatalogSource: Send + Sync {
    // Fetch one page of the whole catalogue, `page` being 1-based.
    fn fetch_catalog_page(&self, page: u32, per_page: u32) -> Result<CatalogPage, String>;

    // Fetch one ranking rail, most-relevant first.
    fn fetch_rail(&self, kind: RailKind) -> Result<Vec<AppEntry>, String>;

    // Fetch a page while allowing the blocking owner to stop the transport.
    fn fetch_catalog_page_cancellable(
        &self,
        page: u32,
        per_page: u32,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<CatalogPage, String> {
        if is_cancelled() {
            return Err(crate::services::CANCELLED_SUMMARY.to_owned());
        }
        self.fetch_catalog_page(page, per_page)
    }

    // Fetch a ranking rail while allowing the blocking owner to stop transport.
    fn fetch_rail_cancellable(
        &self,
        kind: RailKind,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<AppEntry>, String> {
        if is_cancelled() {
            return Err(crate::services::CANCELLED_SUMMARY.to_owned());
        }
        self.fetch_rail(kind)
    }
}

/// The production source: Flathub's v2 collection endpoints over the shared
/// policy-safe HTTP transport.
///
/// No per-app request exists in this type — there is no `fetch_app` method to
/// call in a loop.
pub(crate) struct HttpCatalogSource {
    transport: Box<HttpTransport>,
}

/// The transport seam through which [`HttpCatalogSource`] issues metadata GETs.
///
/// Production wires [`crate::services::http_get_cancellable`]. Tests inject
/// responses here so both producer boundaries exercise the same translation.
type HttpTransport = dyn Fn(&str, &dyn Fn() -> bool) -> Result<String, crate::services::HttpFetchError>
    + Send
    + Sync;

impl HttpCatalogSource {
    // The production source over Flathub's v2 collection endpoints.
    pub(crate) fn new() -> Self {
        Self {
            transport: Box::new(crate::services::http_get_cancellable),
        }
    }

    // Inject a transport so tests drive the real producer translation.
    #[cfg(test)]
    pub(crate) fn with_transport(
        transport: impl Fn(&str) -> Result<String, crate::services::HttpFetchError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            transport: Box::new(move |url, _is_cancelled| transport(url)),
        }
    }
}

impl Default for HttpCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

// Translate a metadata transport failure at the ingest boundary.
//
// The fixed category/code diagnostic is routed to [`log_http_diagnostic`];
// the value handed to [`IngestError`] (and thus every UI
// surface) is the bounded, user-safe [`HttpFetchError::user_summary`]. This
// keeps transport details and remote-controlled data off the Discover status
// page.
fn translate_transport_error(error: crate::services::HttpFetchError) -> String {
    log_http_diagnostic(&error);
    error.user_summary().to_owned()
}

// The escaped structured-log consumer for a raw transport diagnostic: the full
// fixed category/code detail is retained for an operator. No URL, query,
// header, body, connected address, or native error text enters this value.
pub(crate) fn escaped_http_diagnostic(error: &crate::services::HttpFetchError) -> String {
    error.diagnostic().escape_default().collect()
}

fn log_http_diagnostic(error: &crate::services::HttpFetchError) {
    eprintln!(
        "store: catalogue transport failed [{:?}]: {}",
        error.category(),
        escaped_http_diagnostic(error)
    );
}

// Translate a catalogue **payload** parse failure at the ingest boundary.
//
// Structurally identical to [`translate_transport_error`]: the raw
// `serde_json` detail is routed **only** to the escaped structured-log sink,
// and the value handed to [`IngestError`] (and thus Discover) is the bounded,
// user-safe [`crate::services::CatalogParseError::user_summary`]. This keeps
// parser vocabulary, line/column data, and any attacker-controlled
// control/bidi snippet echoed from the body off every UI surface.
fn translate_parse_error(error: crate::services::CatalogParseError) -> String {
    log_parse_diagnostic(&error);
    error.user_summary().to_owned()
}

// The escaped structured-log consumer for a raw parse diagnostic: the full
// `serde_json` detail is retained for an operator, but `str::escape_default`
// guarantees no control character, ANSI escape, bidi override, or newline from
// the payload reaches the log line (or a terminal reading it).
pub(crate) fn escaped_parse_diagnostic(error: &crate::services::CatalogParseError) -> String {
    error.diagnostic().escape_default().collect()
}

fn log_parse_diagnostic(error: &crate::services::CatalogParseError) {
    eprintln!(
        "store: catalogue payload rejected [{:?}]: {}",
        error.category(),
        escaped_parse_diagnostic(error)
    );
}

impl HttpCatalogSource {
    fn fetch_catalog_page_with_cancel(
        &self,
        page: u32,
        per_page: u32,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<CatalogPage, String> {
        let url = format!(
            "https://flathub.org/api/v2/collection/{}?page={page}&per_page={per_page}",
            RailKind::Popular.endpoint()
        );
        let raw = (self.transport)(&url, is_cancelled).map_err(translate_transport_error)?;
        let page = crate::services::parse_collection_page(&raw).map_err(translate_parse_error)?;
        let received_hits = u32::try_from(page.hits.len()).unwrap_or(u32::MAX);
        Ok(CatalogPage {
            entries: page
                .hits
                .iter()
                .filter_map(crate::services::hit_to_entry)
                .collect(),
            received_hits,
            page: page.page,
            total_pages: page.total_pages,
            total_hits: page.total_hits,
        })
    }

    fn fetch_rail_with_cancel(
        &self,
        kind: RailKind,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<AppEntry>, String> {
        let url = format!(
            "https://flathub.org/api/v2/collection/{}?page=1&per_page={RAIL_CAP}",
            kind.endpoint()
        );
        let raw = (self.transport)(&url, is_cancelled).map_err(translate_transport_error)?;
        let page = crate::services::parse_collection_page(&raw).map_err(translate_parse_error)?;
        Ok(page
            .hits
            .iter()
            .filter_map(crate::services::hit_to_entry)
            .collect())
    }
}

impl CatalogSource for HttpCatalogSource {
    fn fetch_catalog_page(&self, page: u32, per_page: u32) -> Result<CatalogPage, String> {
        self.fetch_catalog_page_with_cancel(page, per_page, &|| false)
    }

    fn fetch_rail(&self, kind: RailKind) -> Result<Vec<AppEntry>, String> {
        self.fetch_rail_with_cancel(kind, &|| false)
    }

    fn fetch_catalog_page_cancellable(
        &self,
        page: u32,
        per_page: u32,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<CatalogPage, String> {
        self.fetch_catalog_page_with_cancel(page, per_page, is_cancelled)
    }

    fn fetch_rail_cancellable(
        &self,
        kind: RailKind,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<AppEntry>, String> {
        self.fetch_rail_with_cancel(kind, is_cancelled)
    }
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// A rail as it is cached: a key plus the app ids it ranks.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct RailList {
    key: String,
    ids: Vec<String>,
}

/// The local catalogue: every known app, once, plus the rails that order
/// Discover.
///
/// `entries` is deduplicated by app id **by construction** ([`CatalogIndex::build`]
/// is the only way to fill one), and the rails hold ids into it, so an app in
/// three rails is one entry. `by_id` and `content_hash` are derived and
/// therefore not serialised; [`CatalogIndex::reindex`] rebuilds them after a
/// load.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct CatalogIndex {
    entries: Vec<AppEntry>,
    rails: Vec<RailList>,
    /// Unix seconds when this index was ingested. 0 for an empty index.
    fetched_at: u64,
    /// `totalHits` the server reported, so a partial ingest is detectable.
    total_hits: u32,
    #[serde(skip)]
    by_id: HashMap<String, usize>,
    #[serde(skip)]
    content_hash: u64,
}

impl PartialEq for CatalogIndex {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
            && self.rails == other.rails
            && self.fetched_at == other.fetched_at
            && self.total_hits == other.total_hits
    }
}

impl CatalogIndex {
    /// Build an index from entries and rails, rejecting invalid application
    /// IDs and deduplicating by ID.
    ///
    /// The first copy of an id wins (catalogue order is install-ranked, so the
    /// winner is the better-ranked one). Rail ids that name no entry are
    /// dropped, so a rail can never resurrect an app the index does not hold.
    pub(crate) fn build(
        entries: impl IntoIterator<Item = AppEntry>,
        rails: impl IntoIterator<Item = (RailKind, Vec<String>)>,
        fetched_at: u64,
        total_hits: u32,
    ) -> Self {
        let mut deduped: Vec<AppEntry> = Vec::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        for entry in entries {
            if crate::flatpak_args::validate_app_id(&entry.id).is_err()
                || by_id.contains_key(&entry.id)
            {
                continue;
            }
            by_id.insert(entry.id.clone(), deduped.len());
            deduped.push(entry);
        }
        let rails = rails
            .into_iter()
            .map(|(kind, ids)| {
                let mut seen: Vec<String> = Vec::new();
                for id in ids {
                    if by_id.contains_key(&id) && !seen.contains(&id) {
                        seen.push(id);
                    }
                }
                RailList {
                    key: kind.key().to_owned(),
                    ids: seen,
                }
            })
            .collect();
        let mut index = Self {
            entries: deduped,
            rails,
            fetched_at,
            total_hits,
            by_id,
            content_hash: 0,
        };
        index.content_hash = crate::projection::hash_entries(&index.entries);
        index
    }

    /// The same index stamped with a different ingest time.
    ///
    /// Used by the capture fixture to present a *deterministically* old cache:
    /// the stale banner names the cache's age, so a fixed `fetched_at` would
    /// make the copy change every day the capture is taken, and a committed
    /// baseline would drift for no reason (`VISUAL_QA_AGENT.md` — a capture
    /// must be reproducible across machines and dates).
    pub(crate) fn with_fetched_at(mut self, fetched_at: u64) -> Self {
        self.fetched_at = fetched_at;
        self
    }

    /// Rebuild the derived lookup map and content hash.
    ///
    /// Called after a cache load, where `serde` skipped both. Cached rows are
    /// external input too, so invalid and duplicate IDs are removed here.
    pub(crate) fn reindex(&mut self) {
        let mut entries = Vec::with_capacity(self.entries.len());
        let mut by_id = HashMap::new();
        for entry in std::mem::take(&mut self.entries) {
            if crate::flatpak_args::validate_app_id(&entry.id).is_ok()
                && !by_id.contains_key(&entry.id)
            {
                by_id.insert(entry.id.clone(), entries.len());
                entries.push(entry);
            }
        }
        for rail in &mut self.rails {
            let mut seen = Vec::new();
            rail.ids.retain(|id| {
                if !by_id.contains_key(id) || seen.iter().any(|prior| prior == id) {
                    return false;
                }
                seen.push(id.clone());
                true
            });
        }
        self.entries = entries;
        self.by_id = by_id;
        self.content_hash = crate::projection::hash_entries(&self.entries);
    }

    /// Every known app, once. The candidate set for search, Categories and
    /// Browse.
    pub(crate) fn entries(&self) -> &[AppEntry] {
        &self.entries
    }

    /// Project the locally-installed set onto every indexed entry.
    ///
    /// Ingested catalogue entries always carry `installed: false`, so a browse
    /// route can only render a truthful action after this join.
    ///
    /// `installed` participates in [`crate::projection::hash_entries`], so the
    /// content hash is recomputed: a row projection keyed on the stale hash
    /// would otherwise stay warm across the change.
    pub(crate) fn apply_installed(&mut self, installed: &std::collections::HashSet<String>) {
        let mut changed = false;
        for entry in &mut self.entries {
            let now = installed.contains(&entry.id);
            if entry.installed != now {
                entry.installed = now;
                changed = true;
            }
        }
        if changed {
            self.content_hash = crate::projection::hash_entries(&self.entries);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// One app by id, in O(1).
    pub(crate) fn entry(&self, app_id: &str) -> Option<&AppEntry> {
        self.by_id.get(app_id).and_then(|i| self.entries.get(*i))
    }

    /// The ids one rail ranks, in rail order.
    pub(crate) fn rail_ids(&self, kind: RailKind) -> &[String] {
        self.rails
            .iter()
            .find(|r| r.key == kind.key())
            .map(|r| r.ids.as_slice())
            .unwrap_or(&[])
    }

    /// One rail resolved to entries, in rail order.
    pub(crate) fn rail(&self, kind: RailKind) -> Vec<AppEntry> {
        self.rail_ids(kind)
            .iter()
            .filter_map(|id| self.entry(id))
            .cloned()
            .collect()
    }

    /// The four rails in the shape Discover already consumes.
    pub(crate) fn rails_catalogs(&self) -> crate::services::FlathubCatalogs {
        crate::services::FlathubCatalogs {
            popular: self.rail(RailKind::Popular),
            trending: self.rail(RailKind::Trending),
            new: self.rail(RailKind::RecentlyAdded),
            updated: self.rail(RailKind::RecentlyUpdated),
        }
    }

    /// When this index was ingested (Unix seconds).
    pub(crate) fn fetched_at(&self) -> u64 {
        self.fetched_at
    }

    /// Precomputed content hash for invalidation without rehashing entries on every message.
    pub(crate) fn content_hash(&self) -> u64 {
        self.content_hash
    }

    /// Age in seconds relative to `now`, saturating.
    pub(crate) fn age_secs(&self, now: u64) -> u64 {
        now.saturating_sub(self.fetched_at)
    }

    /// Whether this index is older than `ttl_secs`.
    pub(crate) fn is_stale(&self, now: u64, ttl_secs: u64) -> bool {
        self.age_secs(now) > ttl_secs
    }

    /// Merge bounded, decoded remote icons onto entries that lack one.
    pub(crate) fn apply_icons(
        &mut self,
        icons: &HashMap<String, crate::bounded_image::RemoteImage>,
    ) {
        let mut changed = false;
        for entry in &mut self.entries {
            if entry.icon_image.is_none()
                && let Some(icon) = icons.get(&entry.id)
            {
                entry.icon_image = Some(crate::bounded_image::RenderImage::Remote(icon.clone()));
                changed = true;
            }
        }
        // The hash keys the row projections: lists already projected from
        // this index pick the new icons up only if it moves.
        if changed {
            self.content_hash = crate::projection::hash_entries(&self.entries);
        }
    }
}

impl enzui_core::persist::Persistable for CatalogIndex {
    fn persist_serialize(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    fn persist_deserialize(body: &str) -> Result<Self, String> {
        let mut index: Self = serde_json::from_str(body).map_err(|e| e.to_string())?;
        index.reindex();
        Ok(index)
    }

    fn persist_default() -> Self {
        Self::default()
    }
}

/// The catalogue cache beneath the validated cache root:
/// `<cache-root>/store-app/catalog`.
///
/// Returns `None` when the cache tier is unavailable, in which case the app runs
/// cacheless rather than re-deriving a path from the environment or writing
/// somewhere arbitrary. A cache root that was replaced by pathname since capture
/// is refused, so the framework file backend never opens a substituted root.
pub(crate) fn cache_store(
    cache: Option<&crate::roots::CacheCapability>,
) -> Option<enzui_core::persist::Store<CatalogIndex>> {
    use enzui_core::persist::{FileBackend, Store};
    let cache = cache?;
    if cache.substituted() {
        return None;
    }
    let path = cache.catalog_store_path().to_path_buf();
    let backend = FileBackend::new(path, CATALOG_FRAME_LIMIT);
    Some(Store::new(backend, CACHE_FORMAT_ID, CACHE_VERSION))
}

// ---------------------------------------------------------------------------
// Ingestion
// ---------------------------------------------------------------------------

/// What one complete ingest did, including the number it is judged on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IngestReport {
    pub index: CatalogIndex,
    /// HTTP requests issued. `ceil(N / CATALOG_PAGE_SIZE) + 3`, never a
    /// function of the app count.
    pub requests: u32,
}

/// A failed ingest: the catalogue itself could not be read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IngestError {
    pub message: String,
    pub requests: u32,
}

/// Ingest the whole catalogue plus the three fetched rails.
///
/// Request budget: one per catalogue *page* (`totalPages`, capped by
/// [`MAX_CATALOG_PAGES`]) plus one per rail in [`RailKind::FETCHED`]. Nothing
/// here can issue a request per app — there is no per-app call in
/// [`CatalogSource`] to make.
///
/// Any failed page, inconsistent pagination envelope, or failed rail rejects
/// the ingest, so callers can retain the previous complete index.
pub(crate) fn ingest(source: &dyn CatalogSource, now: u64) -> Result<IngestReport, IngestError> {
    ingest_cancellable(source, now, &|| false)
}

pub(crate) fn ingest_cancellable(
    source: &dyn CatalogSource,
    now: u64,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<IngestReport, IngestError> {
    let mut requests = 0u32;
    let mut entries: Vec<AppEntry> = Vec::new();
    let mut total_hits = 0u32;
    let mut received_hits = 0u64;
    let mut page = 1u32;
    let mut total_pages = 1u32;

    loop {
        if is_cancelled() {
            return Err(IngestError {
                message: crate::services::CANCELLED_SUMMARY.to_owned(),
                requests,
            });
        }
        requests += 1;
        let fetched = source
            .fetch_catalog_page_cancellable(page, CATALOG_PAGE_SIZE, is_cancelled)
            .map_err(|message| IngestError { message, requests })?;
        if fetched.page != page {
            return Err(incomplete_ingest(requests));
        }
        if page == 1 {
            if fetched.total_hits == 0
                || fetched.total_pages == 0
                || fetched.total_pages > MAX_CATALOG_PAGES
            {
                return Err(incomplete_ingest(requests));
            }
            total_hits = fetched.total_hits;
            total_pages = fetched.total_pages;
        } else if fetched.total_hits != total_hits || fetched.total_pages != total_pages {
            return Err(incomplete_ingest(requests));
        }
        received_hits = received_hits.saturating_add(u64::from(fetched.received_hits));
        if fetched.received_hits == 0 && page < total_pages {
            return Err(incomplete_ingest(requests));
        }
        entries.extend(fetched.entries);
        if page >= total_pages {
            break;
        }
        page += 1;
    }
    if received_hits != u64::from(total_hits) {
        return Err(incomplete_ingest(requests));
    }

    let mut rails: Vec<(RailKind, Vec<String>)> = Vec::new();

    // The popular rail is page 1's own order — install-ranked by the endpoint.
    // Free: no request.
    rails.push((
        RailKind::Popular,
        entries
            .iter()
            .take(RAIL_CAP)
            .map(|e| e.id.clone())
            .collect(),
    ));

    for kind in RailKind::FETCHED {
        if is_cancelled() {
            return Err(IngestError {
                message: crate::services::CANCELLED_SUMMARY.to_owned(),
                requests,
            });
        }
        requests += 1;
        let rail = source
            .fetch_rail_cancellable(kind, is_cancelled)
            .map_err(|message| IngestError { message, requests })?
            .into_iter()
            .take(RAIL_CAP)
            .collect::<Vec<_>>();
        let ids = rail.iter().map(|entry| entry.id.clone()).collect();
        // A rail app the catalogue page missed still belongs in the universe;
        // `CatalogIndex::build` deduplicates it by application id.
        entries.extend(rail);
        rails.push((kind, ids));
    }

    Ok(IngestReport {
        index: CatalogIndex::build(entries, rails, now, total_hits),
        requests,
    })
}

fn incomplete_ingest(requests: u32) -> IngestError {
    IngestError {
        message: "Flathub returned an incomplete catalogue.".to_owned(),
        requests,
    }
}

/// The result of one catalogue load, whatever happened.
///
/// Carries the *state the UI must state*: which index to render, whether it is
/// a stale cache being shown because the network failed, the ingest error to
/// show when there is nothing cached to fall back on, and any independent safe
/// warning produced while opening the persisted cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CatalogLoad {
    pub index: CatalogIndex,
    /// True when this index came from the cache after a failed or skipped
    /// refresh — the UI must say the catalogue is not current.
    pub stale: bool,
    /// Set when the refresh failed. Rendered as an actionable error only when
    /// the index is also empty.
    pub error: Option<String>,
    /// User-safe feedback when the persisted cache could not be read or saved.
    /// Kept separate from `error` so a simultaneous ingest failure is not lost.
    pub cache_load_error: Option<String>,
    /// HTTP requests the refresh issued (0 for a pure cache hit).
    pub requests: u32,
}

impl CatalogLoad {
    /// A fresh index straight from the network.
    pub(crate) fn fresh(index: CatalogIndex, requests: u32) -> Self {
        Self {
            index,
            stale: false,
            error: None,
            cache_load_error: None,
            requests,
        }
    }

    /// A cached index shown because the refresh failed.
    pub(crate) fn stale(index: CatalogIndex, error: String, requests: u32) -> Self {
        Self {
            index,
            stale: true,
            error: Some(error),
            cache_load_error: None,
            requests,
        }
    }
}
