//! Catalogue collections and detail tasks.

use super::*;

use iced::Task;

impl Store {
    /// Category counts over unique catalogue and installed app IDs, keyed lowercase.
    pub(crate) fn category_counts(&self) -> std::collections::HashMap<String, usize> {
        let mut counts = std::collections::HashMap::new();

        for e in self.catalog_candidates() {
            if !e.category.is_empty() {
                *counts.entry(e.category.to_ascii_lowercase()).or_insert(0) += 1;
            }
        }
        counts
    }

    // Per-app HTTP lookups are restricted to IDs already present in the public catalogue.
    pub(crate) fn detail_metadata_allowed(&self, app_id: &str) -> bool {
        self.catalog_entry(app_id).is_some()
    }

    pub(crate) fn installed_ids(&self) -> Vec<String> {
        self.installed.iter().map(|e| e.id.clone()).collect()
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.loading_installed || self.loading_catalog
    }

    // Modal hosts own Tab traversal; the outer ring must not move focus a second time.
    pub(crate) fn a_modal_is_open(&self) -> bool {
        self.install_consent.is_some()
            || self.pending_uninstall.is_some()
            || self.pending_remote_removal.is_some()
            || self.permissions_dialog_open
            || self.repositories_focus.is_some()
    }

    // Indexed metadata also covers installed apps absent from every editorial rail.
    pub(crate) fn enrich_installed(&mut self) {
        for row in &mut self.installed {
            let metadata = self.catalog_index.entry(&row.id).or_else(|| {
                self.catalog
                    .popular
                    .iter()
                    .chain(self.catalog.trending.iter())
                    .chain(self.catalog.new.iter())
                    .chain(self.catalog.updated.iter())
                    .find(|entry| entry.id == row.id)
            });
            if let Some(metadata) = metadata {
                if row.icon_image.is_none() {
                    row.icon_image = metadata.icon_image.clone();
                }
                if row.summary.is_empty() {
                    row.summary.clone_from(&metadata.summary);
                }
            }
        }
        self.sync_catalog_installed_flags();
    }

    // The rails hold clones of index entries, so both copies are flagged or
    // Discover and the index-backed search disagree about the same app.
    fn sync_catalog_installed_flags(&mut self) {
        let installed: std::collections::HashSet<String> =
            self.installed.iter().map(|e| e.id.clone()).collect();
        self.catalog_index.apply_installed(&installed);
        for rail in [
            &mut self.catalog.popular,
            &mut self.catalog.trending,
            &mut self.catalog.new,
            &mut self.catalog.updated,
        ] {
            for entry in rail.iter_mut() {
                entry.installed = installed.contains(&entry.id);
            }
        }
    }

    // Routed and boot-time detail opens share these non-mutating reads so their
    // hydrated state cannot drift.
    #[allow(
        clippy::too_many_arguments,
        reason = "Dispatch independent reads with the same captured detail identity"
    )]
    pub(crate) fn detail_open_tasks(
        cache: Option<roots::CacheCapability>,
        app_id: String,
        origin: String,
        scope: Option<services::MutationScope>,
        scope_identity: String,
        metadata_authorized: bool,
        generation: u64,
        cancellation: services::OperationCancellation,
        read_pool: enzui_core::blocking_task::BlockingTaskDispatcher,
    ) -> Vec<Task<Message>> {
        let perm_app = app_id.clone();
        let perm_label = app_id.clone();
        let ctx_app = app_id.clone();
        let ctx_label = app_id;
        let ctx_cache = cache;
        let permission_cancellation = cancellation.clone();
        let context_cancellation = cancellation;
        let ctx_origin = origin;
        let ctx_scope = scope;
        let ctx_scope_identity = scope_identity;
        let permission_task = read_pool.submit(generation, move |ctx| {
            services::read_permissions_cancellable(
                &perm_app,
                permission_cancellation.with_blocking_context(ctx),
            )
        });
        let permission_refused = perm_label.clone();
        let context_task = read_pool.submit(generation, move |ctx| {
            services::fetch_detail_context_cancellable(
                ctx_cache.as_ref(),
                &ctx_app,
                &ctx_origin,
                ctx_scope,
                &ctx_scope_identity,
                metadata_authorized,
                generation,
                context_cancellation.with_blocking_context(ctx),
            )
        });
        let context_refused = ctx_label.clone();
        vec![
            read_pool_task(
                permission_task,
                move |result| Message::PermissionsLoaded {
                    app_id: perm_label,
                    generation,
                    result,
                },
                move |_| Message::PermissionsLoaded {
                    app_id: permission_refused,
                    generation,
                    result: Err("detail read unavailable".to_owned()),
                },
            ),
            read_pool_task(
                context_task,
                move |context| Message::DetailContextLoaded {
                    app_id: ctx_label,
                    generation,
                    context: Box::new(context),
                },
                move |_| Message::DetailContextLoaded {
                    app_id: context_refused,
                    generation,
                    context: Box::new(services::DetailContext::failed()),
                },
            ),
        ]
    }
}
