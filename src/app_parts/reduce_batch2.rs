//! Second reducer batch: permission dialogs, toggles, search.

use super::*;

use iced::Task;

impl Store {
    pub(crate) fn batch2_open_permissions_dialog(&mut self) -> Task<Message> {
        self.open_permissions_dialog()
    }

    pub(crate) fn batch2_close_permissions_dialog(&mut self) -> Task<Message> {
        self.dismiss_permissions_dialog(enzui_core::modal::DismissCause::CloseButton)
    }

    pub(crate) fn batch2_toggle_ratings_breakdown(&mut self) -> Task<Message> {
        self.ratings_breakdown_open = !self.ratings_breakdown_open;
        Task::none()
    }

    pub(crate) fn batch2_permissions_dialog_event(
        &mut self,
        event: enzui_widgets::modal_host::ModalHostEvent,
    ) -> Task<Message> {
        self.reduce_permissions_host(event)
    }

    pub(crate) fn batch2_open_sandbox_dialog(&mut self) -> Task<Message> {
        // Its edits need the permission service, which the page offers it with.
        if !self.permission_service {
            return Task::none();
        }
        self.clear_permission_error();
        self.shared_permission_details = false;
        let focus = self.open_sandbox_dialog();
        let load = self.dispatch_permission_toggles_load();
        Task::batch([focus, load])
    }

    pub(crate) fn batch2_sandbox_permissions_retry(&mut self) -> Task<Message> {
        self.dispatch_permission_toggles_load()
    }

    pub(crate) fn batch2_owned_permission_toggles_loaded(
        &mut self,
        request: u64,
        app_id: String,
        generation: u64,
        outcome: services::PermissionTogglesOutcome,
    ) -> Task<Message> {
        self.deferred_owned_reads.retry_settled(request);
        let current = request == self.permission_toggles_read_request
            && self.detail_read_is_current(&app_id, generation)
            && self.sandbox_dialog_open;
        if current {
            self.permission_toggles = match outcome {
                services::PermissionTogglesOutcome::ReadyPolicy { toggles, snapshot } => {
                    self.shared_permission_snapshot = Some(snapshot);
                    PermissionTogglesState::Ready(toggles)
                }
                services::PermissionTogglesOutcome::Ready(toggles) => {
                    PermissionTogglesState::Ready(toggles)
                }
                services::PermissionTogglesOutcome::Unavailable => {
                    PermissionTogglesState::Unavailable
                }
            };
            self.clear_permission_error();
        }
        self.pump_deferred_owned_read()
    }

    pub(crate) fn batch2_permission_toggles_read_terminated(
        &mut self,
        request: u64,
        app_id: String,
        generation: u64,
        terminal: flatpak::ReadTaskError,
    ) -> Task<Message> {
        self.deferred_owned_reads.retry_settled(request);
        let intent = OwnedReadIntent::PermissionToggles {
            request,
            app_id,
            generation,
        };
        if !self.owned_read_is_current(&intent) {
            return self.pump_deferred_owned_read();
        }
        self.permission_toggles = PermissionTogglesState::Unavailable;
        self.permission_error = Some(read_terminal_text("Permission read", terminal));
        if terminal == flatpak::ReadTaskError::Busy {
            self.retain_deferred_owned_read(intent);
            Task::none()
        } else {
            self.pump_deferred_owned_read()
        }
    }

    pub(crate) fn batch2_toggle_permission(&mut self, key: String, enable: bool) -> Task<Message> {
        let Some(app_id) = self.detail_app_id.clone() else {
            self.permission_error =
                Some("Could not save permission policy: no app is selected.".to_owned());
            self.permission_error_detail = None;
            return Task::none();
        };
        if !self.permission_toggles_ready() {
            // Mutation is disabled unless the editable model is fully
            // known; a stray toggle for an unavailable model is ignored.
            return Task::none();
        }
        let Some(previous) = self
            .permission_toggles()
            .iter()
            .find(|toggle| toggle.key == key)
            .map(|toggle| toggle.enabled)
        else {
            // The machine key is bounded and escaped before it reaches
            // the surface: an unknown key can carry no hostile payload.
            let key = services::flatpak_failure::cap_chars(
                &services::permission_failure::escape_detail(&key),
                64,
            );
            self.permission_error = Some(format!(
                "Could not save permission policy: unknown permission {key}."
            ));
            self.permission_error_detail = None;
            return Task::none();
        };

        // A rejected permission write restores the pre-mutation value and reports its error beside the control.
        if let Some(toggles) = self.permission_toggles_mut()
            && let Some(toggle) = toggles.iter_mut().find(|t| t.key == key)
        {
            toggle.enabled = enable;
        }
        self.clear_permission_error();
        let runner = std::sync::Arc::clone(&self.permission_override_runner);
        let refused_key = key.clone();
        let refused_app = app_id.clone();
        let generation = self.detail_generation;
        let cancellation = self.detail_cancellation.clone();
        let task = self.read_owner.submit(generation, move |context| {
            let runner = services::CancellablePermissionRunner {
                runner: runner.as_ref(),
                cancellation: cancellation.with_blocking_context(context),
            };
            Message::OwnedPermissionMutationFinished {
                app_id: app_id.clone(),
                generation,
                completion: Box::new(permission_override_finished(
                    &runner, &app_id, key, enable, previous,
                )),
            }
        });
        read_pool_task(task, std::convert::identity, move |_error| {
            // The optimistic value was written before dispatch; a pool
            // refusal rolls it back through the same terminal message.
            Message::OwnedPermissionMutationFinished {
                app_id: refused_app,
                generation,
                completion: Box::new(Message::PermissionOverrideFinished {
                    key: refused_key,
                    attempted: enable,
                    previous,
                    result: Err(services::PermissionActionFailure::unavailable(
                        services::PermissionAction::Write,
                    )),
                }),
            }
        })
    }

    pub(crate) fn batch2_permission_override_finished(
        &mut self,
        key: String,
        attempted: bool,
        previous: bool,
        result: Result<(), services::PermissionActionFailure>,
    ) -> Task<Message> {
        match result {
            Ok(()) => self.clear_permission_error(),
            Err(failure) => {
                // Ignore an obsolete completion after a newer click:
                // only the optimistic value written by this request is
                // eligible for rollback.
                if let Some(toggles) = self.permission_toggles_mut()
                    && let Some(toggle) = toggles
                        .iter_mut()
                        .find(|toggle| toggle.key == key && toggle.enabled == attempted)
                {
                    toggle.enabled = previous;
                }
                self.set_permission_failure(&failure);
            }
        }
        if self.permission_override_runner.checked_policy() && self.sandbox_dialog_open {
            return self.dispatch_permission_toggles_load();
        }
        Task::none()
    }

    pub(crate) fn batch2_reset_permissions_to_defaults(&mut self) -> Task<Message> {
        let Some(app_id) = self.detail_app_id.clone() else {
            self.permission_error =
                Some("Could not reset permission policy: no app is selected.".to_owned());
            self.permission_error_detail = None;
            return Task::none();
        };
        let Some(toggles) = self.permission_toggles_mut() else {
            // Mutation is disabled unless the editable model is fully
            // known; a reset for an unavailable model is ignored.
            return Task::none();
        };
        let previous = toggles.clone();
        services::reset_toggles_to_defaults(toggles);
        self.clear_permission_error();
        let runner = std::sync::Arc::clone(&self.permission_override_runner);
        let refused_previous = previous.clone();
        let refused_app = app_id.clone();
        let generation = self.detail_generation;
        let cancellation = self.detail_cancellation.clone();
        let task = self.read_owner.submit(generation, move |context| {
            let runner = services::CancellablePermissionRunner {
                runner: runner.as_ref(),
                cancellation: cancellation.with_blocking_context(context),
            };
            Message::OwnedPermissionMutationFinished {
                app_id: app_id.clone(),
                generation,
                completion: Box::new(permission_reset_finished(&runner, &app_id, previous)),
            }
        });
        read_pool_task(task, std::convert::identity, move |_error| {
            // The optimistic reset was applied before dispatch; a pool
            // refusal restores the prior toggles through the same
            // terminal message.
            Message::OwnedPermissionMutationFinished {
                app_id: refused_app,
                generation,
                completion: Box::new(Message::PermissionResetFinished {
                    previous: refused_previous,
                    result: Err(services::PermissionActionFailure::unavailable(
                        services::PermissionAction::Reset,
                    )),
                }),
            }
        })
    }

    pub(crate) fn batch2_permission_reset_finished(
        &mut self,
        previous: Vec<services::PermissionToggle>,
        result: Result<(), services::PermissionActionFailure>,
    ) -> Task<Message> {
        match result {
            Ok(()) => self.clear_permission_error(),
            Err(failure) => {
                self.permission_toggles = PermissionTogglesState::Ready(previous);
                self.set_permission_failure(&failure);
            }
        }
        if self.permission_override_runner.checked_policy() && self.sandbox_dialog_open {
            return self.dispatch_permission_toggles_load();
        }
        Task::none()
    }

    pub(crate) fn batch2_close_sandbox_dialog(&mut self) -> Task<Message> {
        self.dismiss_sandbox_dialog(enzui_core::modal::DismissCause::CloseButton)
    }

    pub(crate) fn batch2_sandbox_dialog_event(
        &mut self,
        event: enzui_widgets::modal_host::ModalHostEvent,
    ) -> Task<Message> {
        self.reduce_sandbox_host(event)
    }

    pub(crate) fn batch2_search_input(&mut self, q: String) -> Task<Message> {
        let became_non_empty = self.search_query.trim().is_empty() && !q.trim().is_empty();
        self.search_query = q;
        let to_top = Self::scroll_geometry_to_top(&self.search_scroll, SEARCH_LIST_ID);
        if became_non_empty && self.route != Route::Search {
            self.previous_route = self.route;
            self.route = Route::Search;
        }
        if self.search_query.trim().is_empty() && self.route == Route::Search {
            self.route = self.previous_route;
        }
        if self.route != Route::Search {
            return to_top;
        }
        let geometry = self.search_scroll.clone();
        let icons = self.fetch_visible_row_icons(&geometry);
        Task::batch([to_top, icons])
    }

    pub(crate) fn batch2_search_cleared(&mut self) -> Task<Message> {
        self.search_query.clear();
        let to_top = Self::scroll_geometry_to_top(&self.search_scroll, SEARCH_LIST_ID);
        if self.route == Route::Search {
            self.route = self.previous_route;
        }
        to_top
    }

    pub(crate) fn batch2_search_submitted(&mut self) -> Task<Message> {
        match services::push_recent_search_checked(self.recent_searches.clone(), &self.search_query)
        {
            Ok(updated) => {
                self.recent_searches = updated;
                self.recent_search_refusal = None;
                // A newer submission supersedes any reread still in
                // flight so its stale completion cannot overwrite it.
                self.recent_searches_generation = self.recent_searches_generation.wrapping_add(1);
                match self.recent_searches_file.save(&self.recent_searches) {
                    Ok(()) => self.recent_searches_save_error = None,
                    // A blocked file is surfaced by the recovery banner,
                    // not as a save failure.
                    Err(services::RecentSearchesSaveError::Blocked) => {}
                    Err(error) => {
                        self.recent_searches_save_error = Some(error.user_summary());
                    }
                }
            }
            Err(refusal) => self.recent_search_refusal = Some(refusal),
        }
        Task::none()
    }

    pub(crate) fn batch2_recent_searches_retry(&mut self) -> Task<Message> {
        // A disabled store reacquires roots first; the recent-search
        // retry addresses the captured data root, never a re-derived
        // pathname.
        let Some(data) = self.data_cap() else {
            return Task::none();
        };
        let path = data.recent_searches_path();
        // One worker at a time: a retry while a worker is active is a
        // no-op, so repeated retries cannot multiply workers.
        if self.recent_searches_loading {
            return Task::none();
        }
        self.recent_searches_generation = self.recent_searches_generation.wrapping_add(1);
        self.recent_searches_loading = true;
        self.recent_searches_read_busy = false;
        let generation = self.recent_searches_generation;
        let slot = std::sync::Arc::clone(&self.recent_searches_reload_slot);
        // Off-thread through the one bounded read pool. A saturated pool
        // settles this submission as a typed refusal; the reload never
        // reaches the disk and the retry stays available.
        let task = self.read_owner.submit(generation, move |_ctx| {
            let acquire = services::acquire_recent_searches(&path);
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(acquire);
            }
        });
        read_pool_task(
            task,
            move |()| Message::RecentSearchesReloaded { generation },
            move |error| Message::RecentSearchesReloadRefused {
                generation,
                busy: matches!(error, enzui_core::blocking_task::BlockingTaskError::Busy),
            },
        )
    }

    pub(crate) fn batch2_recent_searches_reload_refused(
        &mut self,
        generation: u64,
        busy: bool,
    ) -> Task<Message> {
        // The pool refused this submission; the reload did not run. A
        // superseded epoch is discarded, so a stale refusal cannot clear
        // a newer reload's loading flag or mark it busy.
        if generation == self.recent_searches_generation {
            self.recent_searches_loading = false;
            self.recent_searches_read_busy = busy;
        }
        Task::none()
    }

    pub(crate) fn batch2_recent_searches_reloaded(&mut self, generation: u64) -> Task<Message> {
        let acquire = self
            .recent_searches_reload_slot
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());
        // The single worker finished; a superseded epoch installs
        // neither history nor authority.
        self.recent_searches_loading = false;
        self.recent_searches_read_busy = false;
        if generation == self.recent_searches_generation
            && let Some(acquire) = acquire
        {
            let (history, unavailable) = match acquire.outcome() {
                services::RecentSearchesLoad::Loaded(history) => (Some(history.clone()), None),
                services::RecentSearchesLoad::Unavailable(reason) => (None, Some(*reason)),
            };
            if let Some(history) = history {
                self.recent_searches = history;
            }
            self.recent_searches_unavailable = unavailable;
            self.recent_searches_file.install(acquire);
        }
        Task::none()
    }

    pub(crate) fn batch2_recent_searches_retired(&mut self) -> Task<Message> {
        self.recent_searches_generation = self.recent_searches_generation.wrapping_add(1);
        match self.recent_searches_file.retire(&self.recent_searches) {
            Ok(_identity) => {
                self.recent_searches_unavailable = None;
                self.recent_search_refusal = None;
                self.recent_searches_save_error = None;
            }
            Err(error) => self.recent_searches_save_error = Some(error.user_summary()),
        }
        Task::none()
    }

    pub(crate) fn batch2_library_search_input(&mut self, q: String) -> Task<Message> {
        self.library_query = q;
        Self::scroll_geometry_to_top(&self.library_scroll, LIBRARY_LIST_ID)
    }

    pub(crate) fn batch2_library_search_cleared(&mut self) -> Task<Message> {
        self.library_query.clear();
        Self::scroll_geometry_to_top(&self.library_scroll, LIBRARY_LIST_ID)
    }
}
