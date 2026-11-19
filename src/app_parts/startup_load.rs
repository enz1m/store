//! Startup parsing and catalog loading.

use super::*;

// Startup arguments select views only; unrecognized arguments use Route::default().
pub(crate) fn parse_args(
    args: &[String],
) -> (Route, bool, Option<String>, String, Option<String>, bool) {
    let mut route = Route::default();
    // `repositories` boots with the Repositories dialog open.
    let mut repositories_open = false;
    let mut category = None;
    let mut query = String::new();
    let mut detail = None;
    // Opening the repository form cannot start a mutation.
    let mut add_remote_open = false;

    for arg in args {
        let (key, value) = match arg.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (arg.as_str(), None),
        };
        match key {
            "library" => route = Route::Library,
            "repositories" | "remotes" => repositories_open = true,
            "add-remote" => {
                repositories_open = true;
                add_remote_open = true;
            }
            "search" => {
                route = Route::Search;
                if let Some(v) = value {
                    query = v.to_owned();
                }
            }
            "categories" => {
                route = Route::Categories;
                category = None;
            }
            "category" => {
                route = Route::Categories;
                category = value.map(str::to_ascii_lowercase);
            }
            "discover" => route = Route::Discover,

            "detail" => {
                if let Some(v) = value {
                    route = Route::Detail;
                    detail = Some(v.to_owned());
                }
            }
            _ => {}
        }
    }
    (
        route,
        repositories_open,
        category,
        query,
        detail,
        add_remote_open,
    )
}

// Only the exact "1" enables blocking fixture startup; normal startup remains asynchronous.
pub(crate) fn fixture_settled_env(value: Option<String>) -> bool {
    matches!(value.as_deref(), Some("1"))
}

// Fixture-only boot settles the same reads before the first painted frame.
pub(crate) fn settle_fixture_now(store: &mut Store, hero_env: Option<String>) {
    match services::list_installed_with_fixture(true) {
        Ok(list) => store.installed = list,
        Err(e) => store.last_error = Some(e),
    }
    store.loading_installed = false;

    // Remotes.
    if let Ok(list) = services::list_remotes_with_fixture(true) {
        store.remotes = list;
        store.remotes_load = services::RemoteLoad::Loaded;
    }

    let mut scan = services::list_updates_with_fixture(true);
    let installed: std::collections::HashSet<&str> =
        store.installed.iter().map(|e| e.id.as_str()).collect();
    scan.updates
        .retain(|update| installed.contains(update.app_id.as_str()));
    store.updates_generation = store.updates_generation.wrapping_add(1);
    store.update_discovery = services::UpdateDiscovery::from_scan(
        scan,
        store.updates_generation,
        0,
        &services::UpdateDiscovery::Loading,
    );

    // Catalog — fixture branch short-circuits before the cache read.
    let fixture_cache = store.cache_cap();
    let fixture_load = load_catalog_with_fixture(fixture_cache.as_ref(), true);
    store.adopt_catalog(fixture_load);
    store.loading_catalog = false;
    store.enrich_installed();

    // Extra activity fixtures are opt-in and cannot affect ordinary startup.
    if std::env::var("ENZUI_LIBRARY_REVIEW_FIXTURE").as_deref() == Ok("1") {
        store.operations = services::fixture_operations();
        store.recently_uninstalled = services::fixture_recently_uninstalled();
    }

    let hero_state = featured_image::fixture_hero_state(hero_env.as_deref());
    let ids: Vec<String> = store
        .catalog
        .popular
        .iter()
        .take(3)
        .map(|entry| entry.id.clone())
        .collect();
    for id in ids {
        store.featured_images.insert(id, hero_state.clone());
    }
}

// Detail fixtures settle before the first paint without starting asynchronous reads.
pub(crate) fn settle_detail_fixture_now(store: &mut Store) {
    store.detail_permissions = PermissionRead::Loaded(services::fixture_permissions());
    store.detail_context = Some(services::fixture_detail_context());
    let settled_at = store.activity_now_ms();
    store.detail_settled_at = Some(settled_at);
    store.verification_settled_at = Some(settled_at);
    // The detail capture must exercise the multi-image surface without live
    // HTTP. These are explicitly trusted repository assets, not cache paths.
    store.screenshots = [
        ("Overview", "shots/desktop-light.png"),
        ("Dark appearance", "shots/desktop-dark.png"),
        ("Compact layout", "shots/mobile-light.png"),
    ]
    .into_iter()
    .map(|(caption, path)| services::Screenshot {
        caption: caption.into(),
        image: bounded_image::RenderImage::TrustedBundled(bounded_image::TrustedBundledPath::new(
            path,
        )),
    })
    .collect();
    store.screenshot_viewer = None;
    store.screenshot_load_state = ScreenshotLoadState::Complete;

    let Ok(state) = std::env::var("ENZUI_DETAIL_OPERATION_FIXTURE") else {
        return;
    };
    let Some(app_id) = store.detail_app_id.clone() else {
        return;
    };
    let name = store
        .app_entry(&app_id)
        .map_or_else(|| app_id.clone(), |entry| entry.name);
    if matches!(state.as_str(), "installed" | "removing")
        && !store.installed.iter().any(|entry| entry.id == app_id)
        && let Some(mut entry) = store.app_entry(&app_id)
    {
        entry.installed = true;
        store.installed.push(entry);
    }
    store.operations = match state.as_str() {
        "installing" => vec![model::OperationState {
            attempt: 1,
            cancel: model::CancelState::Idle,
            app_id,
            name,
            kind: model::OpKind::Install,
            progress: 0.42,
            sampled: true,
            message: "Downloading…".into(),
            finished: false,
            error: None,
        }],
        "removing" => vec![model::OperationState {
            attempt: 1,
            cancel: model::CancelState::Idle,
            app_id,
            name,
            kind: model::OpKind::Uninstall,
            progress: 0.58,
            sampled: true,
            message: "Removing…".into(),
            finished: false,
            error: None,
        }],
        "error" => vec![model::OperationState {
            attempt: 1,
            cancel: model::CancelState::Idle,
            app_id,
            name,
            kind: model::OpKind::Install,
            progress: 0.2,
            sampled: true,
            message: "Failed".into(),
            finished: true,
            error: Some("Fixture operation failed".into()),
        }],
        "installed" => Vec::new(),
        _ => panic!("unknown ENZUI_DETAIL_OPERATION_FIXTURE={state}"),
    };
}

// The resolved fixture flag is passed to callees without rereading global environment.
pub(crate) fn load_catalog_with_fixture(
    cache: Option<&roots::CacheCapability>,
    fixture: bool,
) -> catalog::CatalogLoad {
    load_catalog_with_source(
        &catalog::HttpCatalogSource::new(),
        catalog::now_secs(),
        cache,
        fixture,
    )
}

pub(crate) fn load_catalog_with_source(
    source: &dyn catalog::CatalogSource,
    now: u64,
    cache: Option<&roots::CacheCapability>,
    fixture: bool,
) -> catalog::CatalogLoad {
    load_catalog_cached(
        source,
        now,
        catalog::cache_store(cache).as_ref(),
        cache,
        fixture,
    )
}

/// The catalogue to paint from a settled cache read, plus whether a network
/// ingest must follow to refresh a stale or empty cache.
#[derive(Clone, Debug)]
pub(crate) struct CatalogCachePhase {
    pub(crate) paint: catalog::CatalogLoad,
    pub(crate) needs_ingest: bool,
}

// Cached catalogs paint with disk icons; network refreshes run independently.
pub(crate) fn load_catalog_cache_phase(
    cache: Option<roots::CacheCapability>,
    _now: u64,
    fixture: bool,
    is_cancelled: &dyn Fn() -> bool,
) -> CatalogCachePhase {
    if fixture {
        return CatalogCachePhase {
            paint: fixture_catalog_load(),
            needs_ingest: false,
        };
    }
    let store = catalog::cache_store(cache.as_ref());
    let cache_read = read_catalog_cache(store.as_ref());
    if let Some(summary) = cache_read.user_summary.as_deref() {
        eprintln!("catalog cache {:?}: {summary}", cache_read.kind);
    }
    let cache_load_error = cache_read.user_summary;
    let mut cached = cache_read.index;
    hydrate_cached_icons(&mut cached, cache.as_ref(), is_cancelled);
    let mut paint = catalog::CatalogLoad::fresh(cached, 0);
    paint.cache_load_error = cache_load_error;
    CatalogCachePhase {
        paint,
        needs_ingest: true,
    }
}

// [`refresh_catalog`] against an injected source, on the worker thread.
pub(crate) fn refresh_catalog_from(
    cache: Option<roots::CacheCapability>,
    source: std::sync::Arc<dyn catalog::CatalogSource>,
    is_cancelled: &dyn Fn() -> bool,
) -> catalog::CatalogLoad {
    let fixture = services::fixture_enabled();
    refresh_catalog_with_source(
        source.as_ref(),
        catalog::now_secs(),
        catalog::cache_store(cache.as_ref()).as_ref(),
        cache.as_ref(),
        fixture,
        is_cancelled,
    )
}

// Cache load disposition distinguishes usable cached data from absent or refused storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CatalogCacheLoadKind {
    Unavailable,
    Loaded,
    Absent,
    Refused,
    DeserializeFailed,
    Unknown,
}

#[derive(Debug)]
pub(crate) struct CatalogCacheRead {
    pub(crate) index: catalog::CatalogIndex,
    pub(crate) kind: CatalogCacheLoadKind,
    pub(crate) user_summary: Option<String>,
}

pub(crate) fn read_catalog_cache(
    cache: Option<&enzui_core::persist::Store<catalog::CatalogIndex>>,
) -> CatalogCacheRead {
    use enzui_core::persist::LoadOutcome;

    let Some(store) = cache else {
        return CatalogCacheRead {
            index: catalog::CatalogIndex::default(),
            kind: CatalogCacheLoadKind::Unavailable,
            user_summary: None,
        };
    };
    let outcome = store.load();
    let kind = if matches!(&outcome, LoadOutcome::Loaded(_)) {
        CatalogCacheLoadKind::Loaded
    } else if matches!(&outcome, LoadOutcome::Absent { .. }) {
        CatalogCacheLoadKind::Absent
    } else if matches!(&outcome, LoadOutcome::Refused { .. }) {
        CatalogCacheLoadKind::Refused
    } else if matches!(&outcome, LoadOutcome::DeserializeFailed { .. }) {
        CatalogCacheLoadKind::DeserializeFailed
    } else {
        CatalogCacheLoadKind::Unknown
    };
    let user_summary = outcome.user_summary();
    CatalogCacheRead {
        index: outcome.into_value(),
        kind,
        user_summary,
    }
}

pub(crate) fn load_catalog_cached(
    source: &dyn catalog::CatalogSource,
    now: u64,
    cache: Option<&enzui_core::persist::Store<catalog::CatalogIndex>>,
    icons: Option<&roots::CacheCapability>,
    fixture: bool,
) -> catalog::CatalogLoad {
    load_catalog_cached_with_hydrator(source, now, cache, icons, fixture, |index| {
        hydrate_visible_icons(index, icons, &|| false);
    })
}

pub(crate) fn load_catalog_cached_with_hydrator(
    source: &dyn catalog::CatalogSource,
    now: u64,
    cache: Option<&enzui_core::persist::Store<catalog::CatalogIndex>>,
    icons: Option<&roots::CacheCapability>,
    fixture: bool,
    hydrate_cached: impl FnOnce(&mut catalog::CatalogIndex),
) -> catalog::CatalogLoad {
    // Fixtures bypass host caches so captures do not depend on previous launches.
    if fixture {
        return fixture_catalog_load();
    }
    let cache_read = read_catalog_cache(cache);
    if let Some(summary) = cache_read.user_summary.as_deref() {
        eprintln!("catalog cache {:?}: {summary}", cache_read.kind);
    }
    let cache_load_error = cache_read.user_summary;
    let mut cached = cache_read.index;
    if !cached.is_empty() && !cached.is_stale(now, catalog::CATALOG_TTL_SECS) {
        // Persist metadata without renderer handles; hydrate only the initially visible icons.
        hydrate_cached(&mut cached);
        let mut load = catalog::CatalogLoad::fresh(cached, 0);
        load.cache_load_error = cache_load_error;
        return load;
    }
    let mut load = match ingest_catalog_into(source, now, cache, icons, &|| false) {
        Ok(load) => load,
        Err(err) => degrade_to_cache(cached, err),
    };
    if cache_load_error.is_some() {
        load.cache_load_error = cache_load_error;
    }
    load
}

// Stale captures use a fixed age just beyond the catalogue TTL.
pub(crate) const FIXTURE_STALE_AGE_SECS: u64 = 26 * 3600;

pub(crate) fn fixture_catalog_load() -> catalog::CatalogLoad {
    fixture_catalog_load_for(services::fixture_offline())
}

// [`fixture_catalog_load`] with the mode supplied directly, so a test can
// exercise all three states without mutating process-global environment its
// concurrently-running siblings also read.
pub(crate) fn fixture_catalog_load_for(mode: services::FixtureOffline) -> catalog::CatalogLoad {
    let index = catalog::fixture_index();
    match mode {
        services::FixtureOffline::Online => catalog::CatalogLoad::fresh(index, 0),
        // A deterministic age, not the fixture's fixed stamp: the banner names
        // how old the cache is, and `FIXTURE_STALE_AGE_SECS` before *now* reads
        // the same on every machine on every day.
        services::FixtureOffline::Stale => catalog::CatalogLoad::stale(
            index.with_fetched_at(catalog::now_secs().saturating_sub(FIXTURE_STALE_AGE_SECS)),
            "simulated offline: could not resolve flathub.org".into(),
            1,
        ),
        services::FixtureOffline::Cold => catalog::CatalogLoad {
            index: catalog::CatalogIndex::default(),
            stale: false,
            error: Some("simulated offline: could not resolve flathub.org".into()),
            cache_load_error: None,
            requests: 1,
        },
    }
}

// Failed ingestion keeps a known catalogue explicitly stale; without one, return an actionable error.
pub(crate) fn degrade_to_cache(
    cached: catalog::CatalogIndex,
    err: catalog::IngestError,
) -> catalog::CatalogLoad {
    if cached.is_empty() {
        catalog::CatalogLoad {
            index: catalog::CatalogIndex::default(),
            stale: false,
            error: Some(err.message),
            cache_load_error: None,
            requests: err.requests,
        }
    } else {
        catalog::CatalogLoad::stale(cached, err.message, err.requests)
    }
}

pub(crate) fn refresh_catalog_with_source(
    source: &dyn catalog::CatalogSource,
    now: u64,
    cache: Option<&enzui_core::persist::Store<catalog::CatalogIndex>>,
    icons: Option<&roots::CacheCapability>,
    fixture: bool,
    is_cancelled: &dyn Fn() -> bool,
) -> catalog::CatalogLoad {
    if fixture {
        return fixture_catalog_load();
    }
    // Read first even when ingestion succeeds: a refusal must arm the store's
    // write protection before `ingest_catalog_into` attempts to refresh it.
    let cache_read = read_catalog_cache(cache);
    if let Some(summary) = cache_read.user_summary.as_deref() {
        eprintln!("catalog cache {:?}: {summary}", cache_read.kind);
    }
    let cache_load_error = cache_read.user_summary;
    let mut load = match ingest_catalog_into(source, now, cache, icons, is_cancelled) {
        Ok(load) => load,
        Err(err) => {
            let mut cached = cache_read.index;
            hydrate_cached_icons(&mut cached, icons, is_cancelled);
            degrade_to_cache(cached, err)
        }
    };
    if cache_load_error.is_some() {
        load.cache_load_error = cache_load_error;
    }
    load
}

// Serialize and persist catalog metadata on the worker thread, outside the reducer.
pub(crate) fn ingest_catalog_into(
    source: &dyn catalog::CatalogSource,
    now: u64,
    cache: Option<&enzui_core::persist::Store<catalog::CatalogIndex>>,
    icons: Option<&roots::CacheCapability>,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<catalog::CatalogLoad, catalog::IngestError> {
    let report = catalog::ingest_cancellable(source, now, is_cancelled)?;
    let requests = report.requests;
    let mut index = report.index;
    hydrate_visible_icons(&mut index, icons, is_cancelled);
    if is_cancelled() {
        return Err(catalog::IngestError {
            message: services::CANCELLED_SUMMARY.to_owned(),
            requests,
        });
    }
    let mut load = catalog::CatalogLoad::fresh(index, requests);
    if let Some(store) = cache
        && let Err(error) = store.save_blocking(&load.index)
    {
        eprintln!("catalog cache save: {}", error.diagnostic());
        load.cache_load_error = Some("Catalog cache could not be saved".into());
    }
    Ok(load)
}

// Bound initial icon fetching to visible Discover rails and category previews.
pub(crate) const MAX_MEMORY_ICONS: usize = 160;

pub(crate) fn visible_icon_entries(index: &catalog::CatalogIndex) -> Vec<AppEntry> {
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for entry in catalog::RailKind::ALL
        .into_iter()
        .flat_map(|kind| index.rail(kind))
    {
        if seen.insert(entry.id.clone()) {
            entries.push(entry);
        }
    }
    for category in model::BROWSE_CATEGORIES {
        for entry in model::category_rows_from(index.entries().iter(), category)
            .into_iter()
            .take(ui::search::CATEGORY_PREVIEW_CAP)
        {
            if seen.insert(entry.id.clone()) {
                entries.push(entry);
            }
        }
    }
    entries.truncate(MAX_MEMORY_ICONS);
    entries
}

fn hydrate_cached_icons(
    index: &mut catalog::CatalogIndex,
    cache: Option<&roots::CacheCapability>,
    is_cancelled: &dyn Fn() -> bool,
) {
    let Some(icons) = cache.map(roots::CacheCapability::icons) else {
        return;
    };
    hydrate_visible_icons_with(index, |entries| {
        entries
            .iter()
            .filter_map(|entry| {
                if is_cancelled() {
                    return None;
                }
                services::existing_icon_at(icons, &entry.id, &entry.icon_url, is_cancelled)
                    .map(|image| (entry.id.clone(), image))
            })
            .collect()
    });
}

pub(crate) fn hydrate_visible_icons(
    index: &mut catalog::CatalogIndex,
    cache: Option<&roots::CacheCapability>,
    is_cancelled: &dyn Fn() -> bool,
) {
    // With no cache tier there is nowhere to download or read icons; the rails
    // render lettered placeholders until a cache tier is recovered.
    let Some(cache) = cache else {
        return;
    };
    hydrate_visible_icons_with(index, |entries| {
        services::cache_icons_for(cache.icons(), entries, is_cancelled)
    });
}

pub(crate) fn hydrate_visible_icons_with(
    index: &mut catalog::CatalogIndex,
    fetch: impl FnOnce(&[AppEntry]) -> std::collections::HashMap<String, bounded_image::RemoteImage>,
) {
    let entries = visible_icon_entries(index);
    let icons = fetch(&entries);
    index.apply_icons(&icons);
}
