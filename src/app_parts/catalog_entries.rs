//! Catalogue entries and loading.

use super::*;

use iced::Task;

impl Store {
    /// The full [`AppEntry`] for an app id, preferring the installed copy (which
    /// carries the accurate installed `size`/`version`) and falling back to the
    /// catalogue. `None` when the id is in neither — e.g. an app opened by
    /// `detail=<id>` that is not installed and not in the cold catalogue, in
    /// which case the detail page shows only the id and permissions.
    pub(crate) fn app_entry(&self, app_id: &str) -> Option<AppEntry> {
        if let Some(e) = self.installed.iter().find(|e| e.id == app_id) {
            return Some(e.clone());
        }

        self.catalog_entry(app_id).cloned()
    }

    // Prefer the indexed catalogue; rail-only entries remain a fallback.
    pub(crate) fn catalog_entry(&self, app_id: &str) -> Option<&AppEntry> {
        self.enzimos_apps
            .iter()
            .find(|entry| entry.id == app_id)
            .or_else(|| self.flathub_catalog_entry(app_id))
    }

    pub(crate) fn flathub_catalog_entry(&self, app_id: &str) -> Option<&AppEntry> {
        if let Some(e) = self.catalog_index.entry(app_id) {
            return Some(e);
        }
        self.catalog
            .popular
            .iter()
            .chain(self.catalog.trending.iter())
            .chain(self.catalog.new.iter())
            .chain(self.catalog.updated.iter())
            .find(|e| e.id == app_id)
    }

    // Ranking rails contain catalogue IDs so each app retains one authoritative entry.
    pub(crate) fn adopt_catalog(&mut self, load: catalog::CatalogLoad) {
        self.catalog_cache_load_error = load.cache_load_error;
        // A failed refresh with nothing cached must not wipe a catalogue that
        // is already on screen.
        if load.index.is_empty() && !self.catalog_index.is_empty() {
            self.catalog_stale = true;
            self.catalog_error = load.error;
            return;
        }
        self.catalog = load.index.rails_catalogs();
        self.catalog_index = load.index;
        self.catalog_stale = load.stale;
        self.catalog_error = load.error;
        self.enrich_installed();

        // Pages outside the current three are retired — cached handle, retry
        // queue and attempt generation — so the memory bound holds and a load
        // still in flight for a dropped page cannot revive it.
        let current = self.current_featured_ids();
        self.featured_images
            .retain(|app_id, _| current.iter().any(|id| id == app_id));
        self.featured_generations
            .retain(|app_id, _| current.iter().any(|id| id == app_id));
        self.featured_deferred
            .retain(|app_id| current.iter().any(|id| id == app_id));
        if self
            .featured_page
            .as_ref()
            .is_some_and(|page| !current.iter().any(|id| id == page.as_str()))
        {
            self.featured_page = None;
        }
    }

    pub(crate) fn apply_catalog_load(&mut self, load: catalog::CatalogLoad) -> Task<Message> {
        self.loading_catalog = false;
        self.adopt_catalog(load);
        Task::batch([
            self.ensure_initial_featured_image(),
            self.resume_catalog_detail(),
        ])
    }

    // A detail route can open before the catalogue establishes its public ID.
    // Reopen it once authorized; the detail generation retires earlier replies.
    pub(crate) fn resume_catalog_detail(&mut self) -> Task<Message> {
        if self.route == Route::Detail
            && !self.detail_metadata_authorized
            && let Some(app_id) = self.detail_app_id.clone()
            && self.detail_metadata_allowed(&app_id, &self.detail_origin)
        {
            return self.batch1_open_app_detail(app_id);
        }
        Task::none()
    }

    // Cached entries paint before network refresh, including stale entries.
    pub(crate) fn dispatch_catalog_cache_load(&mut self) -> Task<Message> {
        self.catalog_generation = self.catalog_generation.wrapping_add(1);
        let generation = self.catalog_generation;
        self.loading_catalog = true;
        self.catalog_error = None;
        let cache = self.cache_cap();
        let now = catalog::now_secs();
        let fixture = services::fixture_enabled();
        let task = self.catalog_owner.submit(generation, move |ctx| {
            Box::new(load_catalog_cache_phase(cache, now, fixture, &|| {
                ctx.is_cancelled()
            }))
        });
        Task::future(async move {
            match task.await.into_result() {
                Ok(phase) => Message::CatalogCacheLoaded { generation, phase },
                Err(error) => Message::CatalogUnavailable {
                    generation,
                    busy: matches!(error, enzui_core::blocking_task::BlockingTaskError::Busy),
                },
            }
        })
    }

    // Cache loading and its network refresh share one supersession key.
    pub(crate) fn dispatch_catalog_ingest(&mut self, generation: u64) -> Task<Message> {
        self.loading_catalog = true;
        let cache = self.cache_cap();
        let source = std::sync::Arc::clone(&self.catalog_source);
        let task = self.catalog_owner.submit(generation, move |ctx| {
            Box::new(refresh_catalog_from(cache, source, &|| ctx.is_cancelled()))
        });
        Task::future(async move {
            match task.await.into_result() {
                Ok(load) => Message::CatalogIngested { generation, load },
                Err(error) => Message::CatalogUnavailable {
                    generation,
                    busy: matches!(error, enzui_core::blocking_task::BlockingTaskError::Busy),
                },
            }
        })
    }

    // Advance the generation and dispatch a network refresh directly, so a
    // slower boot cache read or ingest still in flight cannot overwrite it.
    pub(crate) fn dispatch_catalog_refresh(&mut self) -> Task<Message> {
        self.catalog_generation = self.catalog_generation.wrapping_add(1);
        let generation = self.catalog_generation;
        self.catalog_error = None;
        self.dispatch_catalog_ingest(generation)
    }

    // Borrow indexed, rail-only and installed entries so cold catalogues retain sideloaded apps.
    pub(crate) fn catalog_candidates(&self) -> Vec<&AppEntry> {
        let mut out: Vec<&AppEntry> = Vec::with_capacity(self.catalog_index.len() + 64);
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for e in self
            .enzimos_apps
            .iter()
            .chain(self.installed.iter().filter(|entry| entry.from_enzimos))
            .chain(self.catalog_index.entries())
            .chain(self.catalog.popular.iter())
            .chain(self.catalog.trending.iter())
            .chain(self.catalog.new.iter())
            .chain(self.catalog.updated.iter())
            .chain(self.installed.iter())
        {
            if seen.insert(e.id.as_str()) {
                out.push(e);
            }
        }
        out
    }

    // How old the shown catalogue is, in whole hours, for the stale banner.
    pub(crate) fn catalog_age_hours(&self) -> u64 {
        if self.catalog_index.fetched_at() == 0 {
            return 0;
        }
        self.catalog_index.age_secs(catalog::now_secs()) / 3600
    }

    // The validated icon for an app id, borrowed from whichever list holds it.
    //
    // An operation carries only an id and a name, and `app_entry` would clone
    // a whole entry, including its decoded image, per row per frame.
    pub(crate) fn app_icon_image(&self, app_id: &str) -> Option<&bounded_image::RenderImage> {
        self.installed
            .iter()
            .find(|e| e.id == app_id)
            .or_else(|| self.catalog_entry(app_id))
            .and_then(|entry| entry.icon_image.as_ref())
    }
}
