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
    pub(crate) fn detail_metadata_allowed(&self, app_id: &str, origin: &str) -> bool {
        origin == services::FLATHUB_REPOSITORY.origin
            && self.flathub_catalog_entry(app_id).is_some()
    }

    pub(crate) fn detail_from_enzimos(&self, app_id: &str) -> bool {
        let origin = services::ENZIMOS_REPOSITORY.origin;
        let scope = match self.detail_scope_identity.as_str() {
            "user" => services::InstallScope::User,
            "system" => services::InstallScope::System,
            "unknown" | "--user" => {
                return self.detail_origin == origin
                    && self.enzimos_apps.iter().any(|entry| entry.id == app_id);
            }
            _ => return false,
        };
        from_enzimos_remote(&self.remotes, &self.detail_origin, &scope)
    }

    pub(crate) fn enzimos_home_entries(&self) -> Vec<&AppEntry> {
        if !self.enzimos_apps.is_empty() {
            return self.enzimos_apps.iter().collect();
        }
        // Fresh offline installs have deployments before the first catalogue refresh.
        let mut seen = std::collections::HashSet::new();
        self.installed
            .iter()
            .filter(|row| {
                from_enzimos_remote(&self.remotes, &row.origin, &row.install_scope)
                    && seen.insert(row.id.as_str())
            })
            .collect()
    }

    pub(crate) fn installed_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.installed.iter().map(|e| e.id.clone()).collect();
        ids.extend(
            crate::system_apps()
                .iter()
                .filter(|id| !self.is_installed(id))
                .cloned(),
        );
        ids
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
            row.from_enzimos = from_enzimos_remote(&self.remotes, &row.origin, &row.install_scope);
            if row.from_enzimos {
                if let Some(metadata) = self.enzimos_apps.iter().find(|entry| entry.id == row.id) {
                    if row.icon_image.is_none() {
                        row.icon_image = metadata.icon_image.clone();
                    }
                    if row.summary.is_empty() {
                        row.summary.clone_from(&metadata.summary);
                    }
                    row.publisher.clone_from(&metadata.publisher);
                    row.description.clone_from(&metadata.description);
                    row.license.clone_from(&metadata.license);
                }
                continue;
            }
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
            self.installed_ids().into_iter().collect();
        self.catalog_index.apply_installed(&installed);
        for rail in [
            &mut self.catalog.popular,
            &mut self.catalog.trending,
            &mut self.catalog.new,
            &mut self.catalog.updated,
            &mut self.enzimos_apps,
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
        &mut self,
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
        let official_context = self.detail_from_enzimos(&app_id).then(|| {
            self.enzimos_apps
                .iter()
                .find(|entry| entry.id == app_id)
                .and_then(|entry| entry.enzimos_detail.as_deref().cloned())
                .unwrap_or_else(|| services::DetailContext {
                    metadata_state: services::DetailRouteState::KnownEmpty,
                    ..Default::default()
                })
        });
        let permission_read = self.detail_permission_read(
            app_id.clone(),
            generation,
            cancellation.clone(),
            read_pool.clone(),
        );
        let ctx_app = app_id.clone();
        let ctx_label = app_id;
        let ctx_cache = cache;
        let context_cancellation = cancellation;
        let ctx_origin = origin;
        let ctx_scope = scope;
        let ctx_scope_identity = scope_identity;
        let context_task = read_pool.submit(generation, move |ctx| {
            if let Some(official) = official_context {
                return official;
            }
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
            permission_read,
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

    // Remote metadata waits for the repository list so it describes the remote an install uses.
    pub(crate) fn detail_permission_read(
        &mut self,
        app_id: String,
        generation: u64,
        cancellation: services::OperationCancellation,
        read_pool: enzui_core::blocking_task::BlockingTaskDispatcher,
    ) -> Task<Message> {
        self.detail_permissions_await_remotes =
            matches!(self.remotes_load, services::RemoteLoad::Loading);
        if self.detail_permissions_await_remotes {
            return Task::none();
        }
        let remote = self.metadata_remote(&app_id);
        let label = app_id.clone();
        let task = read_pool.submit(generation, move |ctx| {
            services::read_permissions_cancellable(
                &app_id,
                remote.as_ref(),
                cancellation.with_blocking_context(ctx),
            )
        });
        let refused = label.clone();
        read_pool_task(
            task,
            move |result| Message::PermissionsLoaded {
                app_id: label,
                generation,
                result,
            },
            move |_| Message::PermissionsLoaded {
                app_id: refused,
                generation,
                result: Err("detail read unavailable".to_owned()),
            },
        )
    }

    pub(crate) fn resume_detail_permission_read(&mut self) -> Task<Message> {
        if !self.detail_permissions_await_remotes {
            return Task::none();
        }
        let Some(app_id) = self.detail_app_id.clone() else {
            self.detail_permissions_await_remotes = false;
            return Task::none();
        };
        self.detail_permission_read(
            app_id,
            self.detail_generation,
            self.detail_cancellation.clone(),
            self.detail_owner.dispatcher(),
        )
    }
}

fn from_enzimos_remote(
    remotes: &[services::RemoteInfo],
    name: &str,
    scope: &services::InstallScope,
) -> bool {
    remotes.iter().any(|remote| {
        remote.name == name
            && services::InstallScope::from_remote_options(&remote.options) == *scope
            && services::matches_catalog_origin(remote, services::ENZIMOS_REPOSITORY.origin)
    })
}
