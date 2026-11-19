//! Owned reads and permission dialogs.

use super::*;

use iced::Task;

impl Store {
    pub(crate) fn allocate_owned_read_request(&mut self) -> Option<u64> {
        let request = self.next_owned_read_request.checked_add(1)?;
        self.next_owned_read_request = request;
        Some(request)
    }

    pub(crate) fn owned_read_is_current(&self, intent: &OwnedReadIntent) -> bool {
        match intent {
            OwnedReadIntent::Installed { request } => *request == self.installed_read_request,
            OwnedReadIntent::Remotes { request } => *request == self.remotes_read_request,
            OwnedReadIntent::Updates {
                request,
                generation,
            } => *request == self.updates_read_request && *generation == self.updates_generation,
            OwnedReadIntent::PermissionToggles {
                request,
                app_id,
                generation,
            } => {
                *request == self.permission_toggles_read_request
                    && self.detail_read_is_current(app_id, *generation)
                    && self.sandbox_dialog_open
            }
        }
    }

    pub(crate) fn retain_deferred_owned_read(&mut self, intent: OwnedReadIntent) {
        if self.owned_read_is_current(&intent) {
            self.deferred_owned_reads.retain_latest(intent);
        }
    }

    pub(crate) fn pump_deferred_owned_read(&mut self) -> Task<Message> {
        if self.closing || self.deferred_owned_reads.retry_in_flight.is_some() {
            return Task::none();
        }
        let snapshot = self.read_owner.dispatcher().snapshot();
        if snapshot.shutdown || snapshot.queued >= snapshot.queue_capacity {
            return Task::none();
        }
        for _ in 0..DeferredOwnedReads::DOMAIN_COUNT {
            let Some(intent) = self.deferred_owned_reads.take_next() else {
                return Task::none();
            };
            if !self.owned_read_is_current(&intent) {
                continue;
            }
            match &intent {
                OwnedReadIntent::Installed { .. } => self.loading_installed = true,
                OwnedReadIntent::Remotes { .. } => {
                    self.remotes_load = services::RemoteLoad::Loading;
                }
                OwnedReadIntent::Updates { .. } => {}
                OwnedReadIntent::PermissionToggles { .. } => {
                    self.permission_toggles = PermissionTogglesState::Loading;
                }
            }
            self.deferred_owned_reads.retry_started(intent.request());
            return self.dispatch_owned_read(intent);
        }
        Task::none()
    }

    pub(crate) fn dispatch_owned_read(&self, intent: OwnedReadIntent) -> Task<Message> {
        let dispatcher = self.read_owner.dispatcher();
        match intent {
            OwnedReadIntent::Installed { request } => flatpak::owned_read_task(
                dispatcher,
                request,
                services::OperationCancellation::default(),
                services::list_installed_cancellable,
                move |result| Message::OwnedInstalledLoaded { request, result },
                move |terminal| Message::InstalledReadTerminated { request, terminal },
            ),
            OwnedReadIntent::Remotes { request } => flatpak::owned_read_task(
                dispatcher,
                request,
                services::OperationCancellation::default(),
                services::list_remotes_cancellable,
                move |result| Message::OwnedRemotesLoaded { request, result },
                move |terminal| Message::RemotesReadTerminated { request, terminal },
            ),
            OwnedReadIntent::Updates {
                request,
                generation,
            } => flatpak::owned_read_task(
                dispatcher,
                request,
                services::OperationCancellation::default(),
                services::list_updates_cancellable,
                move |scan| Message::OwnedUpdatesLoaded {
                    request,
                    generation,
                    scan: Box::new(scan),
                },
                move |terminal| Message::UpdatesReadTerminated {
                    request,
                    generation,
                    terminal,
                },
            ),
            OwnedReadIntent::PermissionToggles {
                request,
                app_id,
                generation,
            } => {
                let ready_label = app_id.clone();
                let terminal_label = app_id.clone();
                let runner = std::sync::Arc::clone(&self.permission_override_runner);
                let scope = match self.installed_scope(&app_id) {
                    services::InstallScope::System => "system",
                    services::InstallScope::User => "user",
                    _ => "unsupported",
                };
                flatpak::owned_read_task(
                    dispatcher,
                    request,
                    self.detail_cancellation.clone(),
                    move |cancellation| runner.read(&app_id, scope, cancellation),
                    move |outcome| Message::OwnedPermissionTogglesLoaded {
                        request,
                        app_id: ready_label,
                        generation,
                        outcome,
                    },
                    move |terminal| Message::PermissionTogglesReadTerminated {
                        request,
                        app_id: terminal_label,
                        generation,
                        terminal,
                    },
                )
            }
        }
    }

    pub(crate) fn start_installed_read(&mut self) -> Task<Message> {
        let Some(request) = self.allocate_owned_read_request() else {
            self.loading_installed = false;
            self.last_error = Some("The installed-app read identity was exhausted.".to_owned());
            return Task::none();
        };
        self.installed_read_request = request;
        self.loading_installed = true;
        self.dispatch_owned_read(OwnedReadIntent::Installed { request })
    }

    pub(crate) fn start_remotes_read(&mut self) -> Task<Message> {
        let Some(request) = self.allocate_owned_read_request() else {
            self.remotes_load = services::RemoteLoad::Failed(
                "The repository read identity was exhausted.".to_owned(),
            );
            return Task::none();
        };
        self.remotes_read_request = request;
        self.remotes_load = services::RemoteLoad::Loading;
        self.dispatch_owned_read(OwnedReadIntent::Remotes { request })
    }

    // Move the editable model to Loading and dispatch a bounded build for the
    // current detail app, tagged with both page generation and read identity.
    pub(crate) fn dispatch_permission_toggles_load(&mut self) -> Task<Message> {
        let Some(app_id) = self.detail_app_id.clone() else {
            return Task::none();
        };
        let Some(request) = self.allocate_owned_read_request() else {
            self.permission_toggles = PermissionTogglesState::Unavailable;
            self.permission_error = Some("The permission read identity was exhausted.".to_owned());
            return Task::none();
        };
        self.shared_permission_snapshot = None;
        self.permission_toggles_read_request = request;
        self.permission_toggles = PermissionTogglesState::Loading;
        self.dispatch_owned_read(OwnedReadIntent::PermissionToggles {
            request,
            app_id,
            generation: self.detail_generation,
        })
    }

    /// Whether the "Sandbox Permissions" editing dialog is open (Cluster B).
    pub(crate) fn sandbox_dialog_open(&self) -> bool {
        self.sandbox_dialog_open
    }

    /// The last rejected permission-policy write, shown in the editing dialog.
    pub(crate) fn permission_error(&self) -> Option<&str> {
        self.permission_error.as_deref()
    }

    // Clear both the authored error surface and its escaped diagnostic sink.
    pub(crate) fn clear_permission_error(&mut self) {
        self.permission_error = None;
        self.permission_error_detail = None;
    }

    // Publish a classified permission failure: the authored surface into
    // [`Self::permission_error`], the escaped raw detail into the sink. The
    // two are split so raw backend payload never enters the semantic copy.
    pub(crate) fn set_permission_failure(&mut self, failure: &services::PermissionActionFailure) {
        self.permission_error = Some(failure.message());
        self.permission_error_detail = failure.diagnostic().map(str::to_owned);
    }

    /// Whether the permissions dialog is currently presented.
    pub(crate) fn permissions_dialog_open(&self) -> bool {
        self.permissions_dialog_open
    }

    pub(crate) fn ratings_breakdown_open(&self) -> bool {
        self.ratings_breakdown_open
    }

    /// A human-readable display name for an app id, resolved from the loaded
    /// catalogue / installed list. Falls back to `None` when the id is unknown
    /// so the caller can show the raw id.
    pub(crate) fn app_display_name(&self, app_id: &str) -> Option<String> {
        self.app_entry(app_id).map(|e| e.name)
    }
}
