//! Store update entry points.

use super::*;

use iced::Task;

impl Store {
    // Centralized projection validation runs even when a routed reducer returns early.
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        crate::i18n::localizer()
            .clone()
            .scope(|| self.localized_update(message))
    }

    pub(crate) fn localized_update(&mut self, message: Message) -> Task<Message> {
        enzui_core::crash_report::observe(&message);
        let task = self.update_routed(message);
        self.sync_projection();
        self.sync_shell_activity();
        self.reconcile_failure_toasts();
        // An unchanged earliest deadline keeps its armed wake.
        self.toasts
            .sync_expiry(&mut self.toast_expiry, std::time::Instant::now());
        task
    }

    pub(crate) fn update_routed(&mut self, message: Message) -> Task<Message> {
        if self.closing
            && !matches!(
                message,
                Message::CloseRetry
                    | Message::AppLaunched(..)
                    | Message::CloseKeepOpen
                    | Message::CloseDrainPoll
                    | Message::OperationEvent(_)
                    | Message::OperationPoll
                    | Message::WindowClosed(_)
                    | Message::Toast(_)
                    | Message::DismissCloseFailure
            )
        {
            return Task::none();
        }
        match message {
            #[cfg(test)]
            Message::InstalledLoaded(result) => self.batch3_installed_loaded(result),
            #[cfg(test)]
            Message::UpdatesLoaded { generation, scan } => {
                self.batch3_updates_loaded(generation, scan)
            }
            #[cfg(test)]
            Message::CatalogLoaded(load) => self.batch4_catalog_loaded(load),

            Message::TitlebarOverflow(open) => {
                self.titlebar_overflow_open = open;
                Task::none()
            }
            Message::Launched => {
                let (_, raise) = self.instance.drain(self.window);
                raise
            }

            Message::LaunchInstalled(app_id) => self.launch_installed(app_id),
            Message::AppLaunched(app_id, request, result) => {
                self.app_launched(app_id, request, result)
            }
            Message::InstallReviewLoaded(generation, result) => {
                self.install_review_loaded(generation, result)
            }
            Message::InstallConsentHost(event) => self.install_consent_host(event),
            Message::InstallConsentCancel => self.cancel_install_consent(),
            Message::InstallConsentConfirm => self.confirm_install_consent(),
            Message::InstallConsentBlock(index, blocked) => {
                self.block_install_permission(index, blocked)
            }
            Message::InstallConsentDetails => {
                if let Some(consent) = self.install_consent.as_mut() {
                    consent.details = !consent.details;
                }
                Task::none()
            }
            Message::ReadOwnerRetryTick => self.batch1_read_owner_retry_tick(),
            // ---- navigation --------------------------------------------
            Message::Navigate(route) => self.batch1_navigate(route),
            Message::Back => self.batch1_back(),
            Message::CategorySelected(key) => self.batch1_category_selected(key),

            Message::OpenAppDetail(app_id) => {
                self.search_popup_open = false;
                self.batch1_open_app_detail(app_id)
            }
            #[cfg(feature = "center")]
            Message::Center(event) => self.center_event(event),
            Message::OpenAppDetailFrom {
                app_id,
                origin,
                install_scope,
            } => {
                self.search_popup_open = false;
                self.batch1_open_app_detail_from(app_id, origin, install_scope)
            }
            Message::PermissionsLoaded {
                app_id,
                generation,
                result,
            } => self.batch1_permissions_loaded(app_id, generation, result),
            Message::PermissionsRetryRequested(app_id) => {
                self.batch1_permissions_retry_requested(app_id)
            }
            Message::DetailContextLoaded {
                app_id,
                generation,
                context,
            } => self.batch1_detail_context_loaded(app_id, generation, context),
            Message::VerificationRetryRequested(app_id) => {
                self.batch1_verification_retry_requested(app_id)
            }
            Message::VerificationLoaded {
                app_id,
                generation,
                verification,
            } => self.batch1_verification_loaded(app_id, generation, verification),
            Message::ScreenshotsLoaded {
                app_id,
                generation,
                result,
            } => self.batch1_screenshots_loaded(app_id, generation, result),
            Message::OpenScreenshot(index) => self.batch1_open_screenshot(index),
            Message::ScreenshotViewerEvent(event) => self.batch1_screenshot_viewer_event(event),
            Message::OpenReleaseHistory => self.batch1_open_release_history(),
            Message::CloseReleaseHistory => self.batch1_close_release_history(),
            Message::ReleaseHistoryEvent(event) => self.batch1_release_history_event(event),
            Message::OpenUrl(url) => self.batch1_open_url(url),
            Message::WindowOpened(id) => self.batch1_window_opened(id),
            Message::ActivityWindowTagged(id, tag, queued) => {
                if self.window == Some(id) {
                    self.shell_activity.confirm_tag(id, tag, queued);
                }
                Task::none()
            }
            Message::WindowClosed(id) => self.batch1_window_closed(id),
            Message::OpenUrlParentExported { url, parent } => {
                self.batch1_open_url_parent_exported(url, parent)
            }
            Message::OpenUrlPortalTick(subscription_generation) => {
                self.batch1_open_url_portal_tick(subscription_generation)
            }
            Message::OpenUrlFinished(outcome) => self.batch1_open_url_finished(outcome),
            Message::OpenPermissionsDialog => self.batch2_open_permissions_dialog(),
            Message::ClosePermissionsDialog => self.batch2_close_permissions_dialog(),
            Message::ToggleRatingsBreakdown => self.batch2_toggle_ratings_breakdown(),
            Message::PermissionsDialogEvent(event) => self.batch2_permissions_dialog_event(event),

            // ---- Sandbox Permissions editing dialog (Cluster B) ----------
            Message::OpenSandboxDialog => self.batch2_open_sandbox_dialog(),
            Message::DialogHostIdle => Task::none(),
            Message::SandboxPermissionsRetry => self.batch2_sandbox_permissions_retry(),

            Message::OwnedPermissionTogglesLoaded {
                request,
                app_id,
                generation,
                outcome,
            } => self.batch2_owned_permission_toggles_loaded(request, app_id, generation, outcome),
            Message::PermissionTogglesReadTerminated {
                request,
                app_id,
                generation,
                terminal,
            } => self
                .batch2_permission_toggles_read_terminated(request, app_id, generation, terminal),
            Message::TogglePermission(key, enable) => self.batch2_toggle_permission(key, enable),
            Message::OwnedPermissionMutationFinished {
                app_id,
                generation,
                completion,
            } => {
                if !self.detail_read_is_current(&app_id, generation) || !self.sandbox_dialog_open {
                    return Task::none();
                }
                match *completion {
                    Message::PermissionOverrideFinished {
                        key,
                        attempted,
                        previous,
                        result,
                    } => self.batch2_permission_override_finished(key, attempted, previous, result),
                    Message::PermissionResetFinished { previous, result } => {
                        self.batch2_permission_reset_finished(previous, result)
                    }
                    _ => Task::none(),
                }
            }
            Message::PermissionOverrideFinished {
                key,
                attempted,
                previous,
                result,
            } => self.batch2_permission_override_finished(key, attempted, previous, result),
            Message::ResetPermissionsToDefaults => self.batch2_reset_permissions_to_defaults(),
            Message::PermissionResetFinished { previous, result } => {
                self.batch2_permission_reset_finished(previous, result)
            }
            Message::CloseSandboxDialog => self.batch2_close_sandbox_dialog(),
            Message::SandboxDialogEvent(event) => self.batch2_sandbox_dialog_event(event),
            Message::SandboxPolicyDetails(expanded) => {
                self.shared_permission_details = expanded;
                Task::none()
            }

            // ---- search --------------------------------------------------
            Message::SearchInput(q) => self.batch2_search_input(q),
            Message::SearchCleared => self.batch2_search_cleared(),
            Message::SearchSubmitted => self.batch2_search_submitted(),
            Message::RecentSearchesRetry => self.batch2_recent_searches_retry(),
            Message::RecentSearchesReloadRefused { generation, busy } => {
                self.batch2_recent_searches_reload_refused(generation, busy)
            }
            Message::RecentSearchesReloaded { generation } => {
                self.batch2_recent_searches_reloaded(generation)
            }
            Message::RecentSearchesRetired => self.batch2_recent_searches_retired(),

            Message::LibrarySearchInput(q) => self.batch2_library_search_input(q),
            Message::LibrarySearchCleared => self.batch2_library_search_cleared(),
            Message::RecentSearchPicked(query) => self.batch3_recent_search_picked(query),
            Message::ToggleVerifiedOnly => self.batch3_toggle_verified_only(),
            Message::ToggleFossOnly => self.batch3_toggle_foss_only(),
            Message::ToggleHideEol => self.batch3_toggle_hide_eol(),
            Message::ToggleInstalledOnly => self.batch3_toggle_installed_only(),

            // ---- discover ------------------------------------------------
            Message::ExpandCollection(kind) => self.batch3_expand_collection(kind),
            Message::FeaturedPageSelected(page) => self.batch3_featured_page_selected(page),
            Message::FeaturedImageLoaded {
                generation,
                outcome,
            } => self.batch3_featured_image_loaded(generation, outcome),
            Message::FeaturedImageRefused {
                app_id,
                generation,
                busy,
            } => self.batch3_featured_image_refused(app_id, generation, busy),
            Message::FeaturedRetryTick => self.batch3_featured_retry_tick(),

            // ---- library -------------------------------------------------
            Message::LibrarySortChanged(sort) => {
                self.library_sort_menu_open = false;
                self.batch3_library_sort_changed(sort)
            }
            Message::LibrarySortMenu(open) => {
                self.library_sort_menu_open = open;
                Task::none()
            }
            Message::AutomaticUpdatesToggled(on) => self.automatic_updates_toggled(on),
            Message::AutomaticUpdatesSaved(on, result) => self.automatic_updates_saved(on, result),
            Message::LibraryScrolled(offset) => self.batch3_library_scrolled(offset),

            // ---- loads ---------------------------------------------------
            Message::RefreshRequested => self.batch3_refresh_requested(),

            Message::OwnedInstalledLoaded { request, result } => {
                self.batch3_owned_installed_loaded(request, result)
            }
            Message::InstalledReadTerminated { request, terminal } => {
                self.batch3_installed_read_terminated(request, terminal)
            }

            Message::OwnedUpdatesLoaded {
                request,
                generation,
                scan,
            } => self.batch3_owned_updates_loaded(request, generation, scan),
            Message::UpdatesReadTerminated {
                request,
                generation,
                terminal,
            } => self.batch3_updates_read_terminated(request, generation, terminal),
            Message::UpdateCheckRetryRequested => self.batch3_update_check_retry_requested(),

            Message::OwnedRemotesLoaded { request, result } => {
                self.batch3_owned_remotes_loaded(request, result)
            }
            Message::RemotesReadTerminated { request, terminal } => {
                self.batch4_remotes_read_terminated(request, terminal)
            }

            Message::CatalogCacheLoaded { generation, phase } => {
                self.batch4_catalog_cache_loaded(generation, phase)
            }
            Message::CatalogIngested { generation, load } => {
                self.batch4_catalog_ingested(generation, load)
            }
            Message::CatalogUnavailable { generation, busy } => {
                self.batch4_catalog_unavailable(generation, busy)
            }

            // ---- operations ----------------------------------------------
            Message::InstallRequested(id, name) => self.batch4_install_requested(id, name),
            Message::UpdateRequested(id, name) => self.batch4_update_requested(id, name),
            Message::UpdateAllRequested => self.batch4_update_all_requested(),

            Message::UninstallConfirmRequested(id, name) => {
                self.batch4_uninstall_confirm_requested(id, name)
            }
            Message::UninstallConfirmEvent(event) => self.batch4_uninstall_confirm_event(event),
            Message::FocusNext => self.batch4_focus_next(),
            Message::FocusPrevious => self.batch4_focus_previous(),
            Message::FocusSearch => self.batch4_focus_search(),
            Message::SearchPopupToggled => {
                if self.search_popup_open {
                    self.search_popup_open = false;
                    Task::none()
                } else {
                    self.open_search_popup()
                }
            }
            Message::SearchPopupDismissed => {
                self.search_popup_open = false;
                Task::none()
            }
            Message::SearchPopupInput(query) => {
                self.search_query = query;
                self.refresh_search_popup()
            }
            Message::SearchPopupShowAll => self.search_popup_show_all(),
            Message::RowIconsLoaded(icons) => {
                self.apply_row_icons(icons);
                Task::none()
            }
            Message::SearchScrolled(offset) => {
                let geometry = self.search_scroll.clone();
                self.list_scrolled(&geometry, offset)
            }
            Message::CategoryScrolled(offset) => {
                let geometry = self.category_scroll.clone();
                self.list_scrolled(&geometry, offset)
            }

            Message::OperationEvent(event) => self.batch4_operation_event(event),
            Message::OperationPoll => self.batch4_operation_poll(),

            Message::CancelOperationRequested(id) => self.batch4_cancel_operation_requested(id),

            Message::ActivityPrune(deadline) => self.batch4_activity_prune(deadline),

            Message::ClearFinishedTasks => self.batch4_clear_finished_tasks(),
            Message::DismissFailedTasks => {
                self.operations
                    .retain(|o| !(o.finished && o.error.is_some()));
                Task::none()
            }
            Message::LibraryActivityToggled => {
                self.library_activity_open = !self.library_activity_open;
                Task::none()
            }
            Message::ClearRecentlyUninstalled => self.batch4_clear_recently_uninstalled(),

            // ---- remotes (cluster 1) --------------------------------------
            Message::AddRemoteOpened => self.batch4_add_remote_opened(),
            Message::AddRemoteNameChanged(name) => self.batch4_add_remote_name_changed(name),
            Message::AddRemoteUrlChanged(url) => self.batch4_add_remote_url_changed(url),
            Message::AddRemoteSubmit => self.batch4_add_remote_submit(),
            Message::AddRemoteCancelled => self.batch4_add_remote_cancelled(),
            Message::AddRemoteDialogEvent(event) => self.batch5_add_remote_dialog_event(event),
            Message::OpenRepositoriesDialog => self.open_repositories_dialog(),
            Message::CloseRepositoriesDialog => {
                self.dismiss_repositories_dialog(enzui_core::modal::DismissCause::CloseButton)
            }
            Message::RepositoriesDialogEvent(event) => self.reduce_repositories_host(event),
            Message::RemoteAddPlanned(generation, result) => {
                self.batch5_remote_add_planned(generation, result)
            }
            Message::AddRemoteReplaceConfirmed => self.batch5_add_remote_replace_confirmed(),
            Message::AddRemoteRenameRequested => self.batch5_add_remote_rename_requested(),
            Message::RemoteReplaced(generation, result) => {
                self.batch5_remote_replaced(generation, result)
            }
            Message::RemoteAdded(generation, result) => {
                self.batch5_remote_added(generation, result)
            }
            Message::RemoveRemoteRequested(name) => self.batch5_remove_remote_requested(name),
            Message::RemoveRemoteConfirmEvent(event) => {
                self.batch5_remove_remote_confirm_event(event)
            }
            Message::RemoteRemoved(name, result) => self.batch5_remote_removed(name, result),

            // ---- shared theme ---------------------------------------------
            Message::Theme(event) => self.batch5_theme(event),

            // Close enters bounded-owner cleanup before the host closes its window.
            Message::WindowGesture(command) => self.batch5_window_gesture(command),
            Message::WindowState(event) => self.chrome.update(event).map(Message::WindowState),
            Message::CloseRequested(id) => self.batch5_close_requested(id),
            Message::CloseRetry => self.batch5_close_retry(),
            Message::CloseKeepOpen => self.batch5_close_keep_open(),
            Message::Toast(event) => {
                if matches!(event, enzui_widgets::toast_host::ToastEvent::Tick(_)) {
                    // Disarm the delivered wake; the reconcile re-arms what remains.
                    self.toast_expiry = enzui_widgets::toast_host::ToastExpirySchedule::default();
                }
                self.toasts.update(event);
                Task::none()
            }
            Message::DismissCloseFailure => {
                self.close_failure_dismissed = self.close_failure_text();
                Task::none()
            }
            Message::CloseDrainPoll => self.batch5_close_drain_poll(),
        }
    }

    // Every update scan gets a generation; retain the displayed answer until the current scan settles.
    pub(crate) fn start_update_check(&mut self) -> Task<Message> {
        let Some(request) = self.allocate_owned_read_request() else {
            self.last_error = Some("The update read identity was exhausted.".to_owned());
            return Task::none();
        };
        self.updates_generation = self.updates_generation.wrapping_add(1);
        let generation = self.updates_generation;
        self.updates_read_request = request;
        self.dispatch_owned_read(OwnedReadIntent::Updates {
            request,
            generation,
        })
    }
}
