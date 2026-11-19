//! Third reducer batch: recent search, remotes, catalog.

use super::*;

use iced::Task;

impl Store {
    pub(crate) fn batch3_recent_search_picked(&mut self, query: String) -> Task<Message> {
        // Re-run a stored query: repopulate the field, route to Search,
        // and refresh the scroll. The query is *not* re-committed here
        // (it is already in the history); it is committed again only if
        // the user submits it afresh.
        self.search_query = query;
        let to_top = Self::scroll_geometry_to_top(&self.search_scroll, SEARCH_LIST_ID);
        if !self.search_query.trim().is_empty() && self.route != Route::Search {
            self.previous_route = self.route;
            self.route = Route::Search;
        }
        to_top
    }

    pub(crate) fn batch3_toggle_verified_only(&mut self) -> Task<Message> {
        self.filters.verified_only = !self.filters.verified_only;
        Task::none()
    }

    pub(crate) fn batch3_toggle_foss_only(&mut self) -> Task<Message> {
        self.filters.foss_only = !self.filters.foss_only;
        Task::none()
    }

    pub(crate) fn batch3_toggle_hide_eol(&mut self) -> Task<Message> {
        self.filters.hide_eol = !self.filters.hide_eol;
        Task::none()
    }

    pub(crate) fn batch3_toggle_installed_only(&mut self) -> Task<Message> {
        self.filters.installed_only = !self.filters.installed_only;
        Task::none()
    }

    pub(crate) fn batch3_expand_collection(
        &mut self,
        kind: model::CollectionKind,
    ) -> Task<Message> {
        self.expanded_collection = Some(kind);
        Task::none()
    }

    pub(crate) fn batch3_featured_page_selected(
        &mut self,
        page: enzui_core::paging::PageId,
    ) -> Task<Message> {
        let app_id = page.as_str().to_owned();
        self.featured_page = Some(page);
        self.ensure_featured_image(app_id)
    }

    pub(crate) fn batch3_featured_image_loaded(
        &mut self,
        generation: u64,
        outcome: featured_image::HeroImageOutcome,
    ) -> Task<Message> {
        if !self.featured_settlement_is_current(&outcome.app_id, generation) {
            return Task::none();
        }
        let state = match outcome.result {
            Ok(handle) => featured_image::HeroImageState::Ready(handle),
            Err(error) => featured_image::HeroImageState::Unavailable(error),
        };
        self.featured_generations.remove(&outcome.app_id);
        self.featured_deferred.retain(|id| id != &outcome.app_id);
        if let Some(brand) = outcome.brand {
            self.featured_brands.insert(outcome.app_id.clone(), brand);
        }
        self.featured_images.insert(outcome.app_id, state);
        self.retry_one_deferred_featured()
    }

    pub(crate) fn batch3_featured_image_refused(
        &mut self,
        app_id: String,
        generation: u64,
        busy: bool,
    ) -> Task<Message> {
        if !self.featured_settlement_is_current(&app_id, generation) {
            return Task::none();
        }
        if busy {
            if !self.featured_deferred.iter().any(|id| id == &app_id) {
                self.featured_deferred.push(app_id);
            }
        } else {
            self.featured_generations.remove(&app_id);
            self.featured_images.insert(
                app_id,
                featured_image::HeroImageState::Unavailable(FEATURED_IMAGE_UNAVAILABLE.to_owned()),
            );
        }
        Task::none()
    }

    pub(crate) fn batch3_featured_retry_tick(&mut self) -> Task<Message> {
        self.retry_one_deferred_featured()
    }

    pub(crate) fn batch3_library_sort_changed(&mut self, sort: LibrarySort) -> Task<Message> {
        self.library_sort = sort;
        let to_top = Self::scroll_geometry_to_top(&self.library_scroll, LIBRARY_LIST_ID);
        self.route = Route::Library;
        to_top
    }

    pub(crate) fn automatic_updates_toggled(&mut self, on: bool) -> Task<Message> {
        let Some(data) = self.capabilities.data().cloned() else {
            self.automatic_updates_error =
                Some(crate::i18n::label("Store cannot save settings here").to_owned());
            return Task::none();
        };
        self.automatic_updates = on;
        self.automatic_updates_error = None;
        let (sender, saved) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(data.set_automatic_updates(on).map_err(|e| e.to_string()));
        });
        Task::future(async move {
            let result = saved.await.unwrap_or_else(|_| Err(String::new()));
            Message::AutomaticUpdatesSaved(on, result)
        })
    }

    pub(crate) fn automatic_updates_saved(
        &mut self,
        on: bool,
        result: Result<(), String>,
    ) -> Task<Message> {
        if result.is_err() && self.automatic_updates == on {
            self.automatic_updates = !on;
            self.automatic_updates_error =
                Some(crate::i18n::label("The setting could not be saved").to_owned());
        }
        Task::none()
    }

    pub(crate) fn batch3_library_scrolled(&mut self, offset: f32) -> Task<Message> {
        // The scrollable re-publishes its viewport when only the
        // surrounding geometry moved, and the retained rows it reports
        // from are rebuilt from inside this same subtree, so an
        // unconditional invalidate re-realises the whole row window on
        // frames nobody scrolled. The realised offset — not the live
        // translation, which the widget has already advanced by now —
        // is what says whether these rows still match.
        if self.library_realised_at.get() == offset {
            return Task::none();
        }
        if let Some(mut measured) = self.library_scroll.get() {
            measured.translation.y = offset;
            self.library_scroll.set(measured);
        }
        self.library_cache.invalidate();
        Task::none()
    }

    pub(crate) fn batch3_refresh_requested(&mut self) -> Task<Message> {
        self.loading_installed = true;
        self.loading_catalog = true;

        self.remotes_load = services::RemoteLoad::Loading;
        self.last_error = None;
        self.close_recovery_error = None;
        let mut tasks = vec![
            self.start_installed_read(),
            self.start_remotes_read(),
            self.start_update_check(),
        ];
        // The catalogue refresh is a network read that runs regardless
        // of the cache tier through the bounded catalogue pool; the
        // cache write is skipped when the cache tier is unavailable.
        tasks.push(self.dispatch_catalog_refresh());
        if self.route == Route::Detail
            && let Some(app_id) = self.detail_app_id.clone()
        {
            self.detail_metadata_authorized = self.detail_metadata_allowed(&app_id);
            let age_secs = self.detail_settled_at.map_or(0, |settled| {
                self.activity_now_ms().saturating_sub(settled) / 1000
            });
            if matches!(
                self.screenshot_load_state,
                ScreenshotLoadState::KnownEmpty
                    | ScreenshotLoadState::Complete
                    | ScreenshotLoadState::Partial
            ) {
                self.screenshot_load_state = ScreenshotLoadState::CachedStale;
            }
            if let Some(context) = self.detail_context.as_mut() {
                let mark_stale = |state: &mut services::DetailRouteState| {
                    if matches!(
                        state,
                        services::DetailRouteState::Known
                            | services::DetailRouteState::KnownEmpty
                            | services::DetailRouteState::CachedStale { .. }
                    ) {
                        *state = services::DetailRouteState::CachedStale { age_secs };
                    }
                };
                mark_stale(&mut context.metadata_state);
                mark_stale(&mut context.stats_state);
                mark_stale(&mut context.ratings_state);
                if let Some(retained) = context.verification.retained(age_secs.saturating_mul(1000))
                {
                    context.verification = retained;
                }
            }
            if let Some(generation) = self.next_detail_generation() {
                tasks.extend(Self::detail_open_tasks(
                    self.cache_cap(),
                    app_id,
                    self.detail_origin.clone(),
                    self.detail_scope,
                    self.detail_scope_identity.clone(),
                    self.detail_metadata_authorized,
                    generation,
                    self.detail_cancellation.clone(),
                    self.detail_owner.dispatcher(),
                ));
            }
        }
        Task::batch(tasks)
    }

    #[cfg(test)]
    pub(crate) fn batch3_installed_loaded(
        &mut self,
        result: Result<Vec<AppEntry>, String>,
    ) -> Task<Message> {
        self.loading_installed = false;
        match result {
            Ok(list) => {
                self.installed = list;
                self.installed_known = true;
                self.enrich_installed();
                self.apply_pending_update_scan();
            }
            Err(e) => self.last_error = Some(e),
        }
        Task::none()
    }

    pub(crate) fn batch3_owned_installed_loaded(
        &mut self,
        request: u64,
        result: Result<Vec<AppEntry>, String>,
    ) -> Task<Message> {
        self.deferred_owned_reads.retry_settled(request);
        if request == self.installed_read_request {
            self.loading_installed = false;
            match result {
                Ok(list) => {
                    self.installed = list;
                    self.installed_known = true;
                    self.enrich_installed();
                    self.apply_pending_update_scan();
                }
                Err(error) => self.last_error = Some(error),
            }
        }
        self.pump_deferred_owned_read()
    }

    pub(crate) fn batch3_installed_read_terminated(
        &mut self,
        request: u64,
        terminal: flatpak::ReadTaskError,
    ) -> Task<Message> {
        self.deferred_owned_reads.retry_settled(request);
        let intent = OwnedReadIntent::Installed { request };
        if !self.owned_read_is_current(&intent) {
            return self.pump_deferred_owned_read();
        }
        self.loading_installed = false;
        self.last_error = Some(read_terminal_text("Installed-app read", terminal));
        if terminal == flatpak::ReadTaskError::Busy {
            self.retain_deferred_owned_read(intent);
            Task::none()
        } else {
            self.pump_deferred_owned_read()
        }
    }

    #[cfg(test)]
    pub(crate) fn batch3_updates_loaded(
        &mut self,
        generation: u64,
        scan: Box<services::UpdateScan>,
    ) -> Task<Message> {
        // Only the newest scan may replace the displayed answer, including its failure state.
        if generation != self.updates_generation {
            return Task::none();
        }
        self.accept_update_scan(generation, scan);
        Task::none()
    }

    /// Filter a scan to installed apps, or hold it until the installed list is
    /// known: filtering against an empty, unloaded list would drop every
    /// update and report "up to date".
    pub(crate) fn accept_update_scan(&mut self, generation: u64, scan: Box<services::UpdateScan>) {
        if !self.installed_known {
            self.pending_update_scan = Some((generation, scan));
            return;
        }
        let mut scan = *scan;
        // `remote-ls --updates` includes runtimes and extensions;
        // the badge should count *apps*, which is what the user
        // can act on from this UI.
        let installed: std::collections::HashSet<&str> =
            self.installed.iter().map(|e| e.id.as_str()).collect();
        // A fresh scan replaces both scopes, including updates that reappear after an earlier settlement.
        scan.updates
            .retain(|update| installed.contains(update.app_id.as_str()));
        let now = self.activity_now_ms();
        // Incomplete scans must retain the timestamp of the last complete check.
        self.update_discovery =
            services::UpdateDiscovery::from_scan(scan, generation, now, &self.update_discovery);
    }

    fn apply_pending_update_scan(&mut self) {
        if let Some((generation, scan)) = self.pending_update_scan.take()
            && generation == self.updates_generation
        {
            self.accept_update_scan(generation, scan);
        }
    }

    pub(crate) fn batch3_owned_updates_loaded(
        &mut self,
        request: u64,
        generation: u64,
        scan: Box<services::UpdateScan>,
    ) -> Task<Message> {
        self.deferred_owned_reads.retry_settled(request);
        if request == self.updates_read_request && generation == self.updates_generation {
            self.accept_update_scan(generation, scan);
        }
        self.pump_deferred_owned_read()
    }

    pub(crate) fn batch3_updates_read_terminated(
        &mut self,
        request: u64,
        generation: u64,
        terminal: flatpak::ReadTaskError,
    ) -> Task<Message> {
        self.deferred_owned_reads.retry_settled(request);
        let intent = OwnedReadIntent::Updates {
            request,
            generation,
        };
        if !self.owned_read_is_current(&intent) {
            return self.pump_deferred_owned_read();
        }
        let failure = match terminal {
            flatpak::ReadTaskError::Cancelled => services::UpdateScanFailure::Cancelled,
            _ => services::UpdateScanFailure::Failed(read_terminal_text("Update read", terminal)),
        };
        let scan = services::UpdateScan::failed(failure);
        let now = self.activity_now_ms();
        self.update_discovery =
            services::UpdateDiscovery::from_scan(scan, generation, now, &self.update_discovery);
        if terminal == flatpak::ReadTaskError::Busy {
            self.retain_deferred_owned_read(intent);
            Task::none()
        } else {
            self.pump_deferred_owned_read()
        }
    }

    pub(crate) fn batch3_update_check_retry_requested(&mut self) -> Task<Message> {
        self.last_error = None;
        self.start_update_check()
    }

    pub(crate) fn batch3_owned_remotes_loaded(
        &mut self,
        request: u64,
        result: Result<Vec<RemoteInfo>, String>,
    ) -> Task<Message> {
        self.deferred_owned_reads.retry_settled(request);
        if request == self.remotes_read_request {
            match result {
                Ok(list) => {
                    self.remotes = list;
                    self.remotes_load = services::RemoteLoad::Loaded;
                }
                Err(error) => {
                    self.remotes_load = services::RemoteLoad::Failed(error);
                }
            }
        }
        self.pump_deferred_owned_read()
    }
}
