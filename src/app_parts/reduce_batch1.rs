//! First reducer batch: navigation, detail reads, screenshots, URLs.

use super::*;

use crate::support::store_export_parent_task;
use iced::Task;

impl Store {
    pub(crate) fn batch1_read_owner_retry_tick(&mut self) -> Task<Message> {
        self.pump_deferred_owned_read()
    }

    pub(crate) fn batch1_navigate(&mut self, route: Route) -> Task<Message> {
        if self.route == Route::Detail && route != Route::Detail {
            self.cancel_detail_requests();
        }
        if route != self.route {
            self.previous_route = self.route;
        }
        if route == Route::Categories {
            self.browse_category = None;
        }

        self.expanded_collection = None;
        self.route = route;

        Task::batch(self.close_detail_modals_for_host())
    }

    pub(crate) fn batch1_back(&mut self) -> Task<Message> {
        if self.expanded_collection.is_some() {
            self.expanded_collection = None;
            return Task::none();
        }
        let next = if self.browse_category.is_some() {
            self.browse_category = None;
            Route::Categories
        } else {
            self.previous_route
        };
        if self.route == Route::Detail && next != Route::Detail {
            self.cancel_detail_requests();
        }
        self.route = next;
        Task::batch(self.close_detail_modals_for_host())
    }

    pub(crate) fn batch1_category_selected(&mut self, key: String) -> Task<Message> {
        self.browse_category = Some(key);
        self.route = Route::Categories;
        let to_top = Self::scroll_geometry_to_top(&self.category_scroll, CATEGORY_LIST_ID);
        let geometry = self.category_scroll.clone();
        let icons = self.fetch_visible_row_icons(&geometry);
        Task::batch([to_top, icons])
    }

    pub(crate) fn batch1_open_app_detail(&mut self, app_id: String) -> Task<Message> {
        let entry = self.app_entry(&app_id).unwrap_or_else(|| AppEntry {
            id: app_id,
            origin: "flathub".to_owned(),
            ..AppEntry::default()
        });
        self.update_routed(Message::open_app_detail(&entry))
    }

    pub(crate) fn batch1_open_app_detail_from(
        &mut self,
        app_id: String,
        origin: String,
        install_scope: services::InstallScope,
    ) -> Task<Message> {
        if self.route != Route::Detail {
            self.previous_route = self.route;
        }
        let origin = if origin.is_empty() {
            "flathub".to_owned()
        } else {
            origin
        };
        let scope = match install_scope {
            services::InstallScope::Unknown => {
                services::install_authorization(&origin, &self.remotes_load, &self.remotes).scope()
            }
            services::InstallScope::User => Some(services::MutationScope::USER),
            services::InstallScope::System | services::InstallScope::Named(_) => None,
        };
        let metadata_authorized = self.detail_metadata_allowed(&app_id, &origin, &install_scope);
        self.route = Route::Detail;
        self.detail_app_id = Some(app_id.clone());
        self.detail_origin = origin.clone();
        self.detail_scope = scope;
        self.detail_install_scope = install_scope;
        self.detail_metadata_authorized = metadata_authorized;

        self.ratings_breakdown_open = false;
        // A fresh detail page drops the previous app's
        // publisher-link error so a stale "could not open" line
        // never bleeds across a navigation.
        self.last_url_error = None;
        // Explicit link launches survive navigation; permissions are loaded off the render thread.
        self.detail_permissions = PermissionRead::Loading;
        let close_permissions = self.close_permissions_for_host();
        let close_detail_modals = self.close_detail_modals_for_host();
        // Fresh detail page: drop the previous app's release history so
        // a stale changelog never bleeds across a navigation.
        self.detail_context = None;
        self.detail_settled_at = None;

        self.verification_settled_at = None;

        self.screenshots = Vec::new();
        self.screenshot_viewer = None;
        self.screenshot_load_state = ScreenshotLoadState::Loading;
        let Some(generation) = self.next_detail_generation() else {
            self.detail_permissions = PermissionRead::Unavailable;
            self.detail_context = Some(services::DetailContext::failed());
            self.screenshot_load_state = ScreenshotLoadState::Failed;
            let mut tasks = vec![Self::scroll_to_top_operation(DETAIL_SCROLL_ID)];
            tasks.extend(close_detail_modals);
            if let Some(close) = close_permissions {
                tasks.push(close);
            }
            return Task::batch(tasks);
        };
        let mut tasks = self.detail_open_tasks(app_id, generation);
        // The detail scrollable keeps widget-owned state across route
        // swaps. Reset it so app B never inherits app A's deep offset.
        tasks.push(Self::scroll_to_top_operation(DETAIL_SCROLL_ID));
        tasks.extend(close_detail_modals);
        if let Some(close) = close_permissions {
            tasks.push(close);
        }
        Task::batch(tasks)
    }

    pub(crate) fn batch1_permissions_loaded(
        &mut self,
        app_id: String,
        generation: u64,
        result: Result<services::Permissions, String>,
    ) -> Task<Message> {
        // A completion for a page the user has left must never publish
        // onto the current page, whether it succeeded or failed: refuse
        // it before touching `PermissionRead`, so app A never shows its
        // permissions (or its failure) as app B's.
        if !self.detail_read_is_current(&app_id, generation) {
            return Task::none();
        }
        self.detail_permissions = match result {
            Ok(perms) => PermissionRead::Loaded(perms),
            // A failed read (no flatpak, app absent, transport or
            // process refusal) stays a typed, retryable Unavailable. It
            // is never an empty permission set, so the pill row shows
            // "unavailable" with a Retry, never the reassuring
            // "no exceptions" line.
            Err(_) => PermissionRead::Unavailable,
        };
        Task::none()
    }

    pub(crate) fn batch1_permissions_retry_requested(&mut self, app_id: String) -> Task<Message> {
        // Only the app on screen: a retry must re-read only its
        // permissions, not re-run the detail enrichment fan-out.
        if !self.detail_read_is_current(&app_id, self.detail_generation)
            || !matches!(self.detail_permissions, PermissionRead::Unavailable)
        {
            return Task::none();
        }
        self.detail_permissions = PermissionRead::Loading;
        // Tag the completion with the generation in force now, so a
        // navigation after this click supersedes the retry and its
        // completion is refused rather than published onto the new page.
        self.detail_permission_read(
            app_id,
            self.detail_generation,
            self.detail_cancellation.clone(),
            self.detail_owner.dispatcher(),
        )
    }

    pub(crate) fn batch1_detail_context_loaded(
        &mut self,
        app_id: String,
        generation: u64,
        context: Box<services::DetailContext>,
    ) -> Task<Message> {
        // Reject stale app visits before applying trust and publisher details.
        if !self.detail_read_is_current(&app_id, generation) {
            return Task::none();
        }
        // A context whose own fetch was cancelled describes a page the
        // user left. It is not the current answer even when the ids
        // match, because the ids match again when the user returns.
        if context.cancelled {
            return Task::none();
        }
        let mut context = *context;
        let screenshot_candidates = std::mem::take(&mut context.screenshot_candidates);
        let metadata_failed = matches!(context.metadata_state, services::DetailRouteState::Failed);
        self.detail_context = Some(context);
        let settled_at = self.activity_now_ms();
        self.detail_settled_at = Some(settled_at);
        self.verification_settled_at = Some(settled_at);
        let Some(cache) = self.cache_cap() else {
            self.apply_screenshot_result(services::ScreenshotFetchResult::Failed);
            return Task::none();
        };
        if metadata_failed {
            self.apply_screenshot_result(services::ScreenshotFetchResult::Failed);
            return Task::none();
        }
        let cancellation = self.detail_cancellation.clone();
        let screenshot_app = app_id.clone();
        let task = self.detail_owner.submit(generation, move |ctx| {
            services::cache_screenshot_records_at(
                &cache,
                &app_id,
                generation,
                &screenshot_candidates,
                &|| ctx.is_cancelled() || cancellation.is_cancelled(),
            )
        });
        let refused_app = screenshot_app.clone();
        read_pool_task(
            task,
            move |result| Message::ScreenshotsLoaded {
                app_id: screenshot_app,
                generation,
                result,
            },
            move |error| Message::ScreenshotsLoaded {
                app_id: refused_app,
                generation,
                result: match error {
                    enzui_core::blocking_task::BlockingTaskError::Cancelled => {
                        services::ScreenshotFetchResult::Cancelled
                    }
                    _ => services::ScreenshotFetchResult::Failed,
                },
            },
        )
    }

    pub(crate) fn batch1_verification_retry_requested(&mut self, app_id: String) -> Task<Message> {
        // Only the app on screen, and only its verification: a retry
        // must not re-run the whole enrichment fan-out, or a user
        // pressing it would also replace the release history and
        // ratings they are reading.
        if !self.detail_read_is_current(&app_id, self.detail_generation)
            || !self.detail_metadata_authorized
            || !self
                .detail_context
                .as_ref()
                .and_then(|context| context.verification.unavailable_reason())
                .is_some_and(|unavailable| unavailable.retry.offers_retry())
        {
            return Task::none();
        }
        if let Some(context) = self.detail_context.as_mut() {
            context.verification = services::PublisherVerification::pending();
        }
        let generation = self.detail_generation;
        let label = app_id.clone();
        let cancellation = self.detail_cancellation.clone();
        let task = self.detail_owner.submit(generation, move |ctx| {
            services::fetch_publisher_verification_cancellable(
                &app_id,
                cancellation.with_blocking_context(ctx),
            )
        });
        let refused = label.clone();
        read_pool_task(
            task,
            move |verification| Message::VerificationLoaded {
                app_id: label,
                generation,
                verification,
            },
            move |_| Message::VerificationLoaded {
                app_id: refused,
                generation,
                verification: services::PublisherVerification::unavailable(
                    services::VerificationFailure::Transport(
                        "detail worker unavailable".to_owned(),
                    ),
                ),
            },
        )
    }

    pub(crate) fn batch1_verification_loaded(
        &mut self,
        app_id: String,
        generation: u64,
        verification: services::PublisherVerification,
    ) -> Task<Message> {
        if !self.detail_read_is_current(&app_id, generation) {
            return Task::none();
        }
        self.apply_verification(verification);
        Task::none()
    }

    pub(crate) fn batch1_screenshots_loaded(
        &mut self,
        app_id: String,
        generation: u64,
        result: services::ScreenshotFetchResult,
    ) -> Task<Message> {
        if !self.detail_read_is_current(&app_id, generation) {
            return Task::none();
        }
        self.apply_screenshot_result(result);
        Task::none()
    }

    pub(crate) fn batch1_open_screenshot(&mut self, index: usize) -> Task<Message> {
        if index >= self.screenshots.len() {
            return Task::none();
        }
        self.screenshot_viewer = Some(index);
        iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::focus(
            enzui_core::identity::derive_id(
                enzui_widgets::full_screen_overlay::DEFAULT_ID_PREFIX,
                "close",
            ),
        ))
    }

    pub(crate) fn batch1_screenshot_viewer_event(
        &mut self,
        event: enzui_widgets::full_screen_overlay::FullScreenOverlayEvent,
    ) -> Task<Message> {
        use enzui_widgets::full_screen_overlay::FullScreenOverlayEvent;
        match event {
            FullScreenOverlayEvent::Dismissed(_) => match self.screenshot_viewer.take() {
                Some(index) => iced::advanced::widget::operate(
                    iced::advanced::widget::operation::focusable::focus(Self::screenshot_strip_id(
                        index,
                    )),
                ),
                None => Task::none(),
            },
            other => other
                .focus_operation::<Message>()
                .map_or_else(Task::none, iced::advanced::widget::operate),
        }
    }

    pub(crate) fn batch1_open_release_history(&mut self) -> Task<Message> {
        self.open_release_history()
    }

    pub(crate) fn batch1_close_release_history(&mut self) -> Task<Message> {
        self.dismiss_release_history(enzui_core::modal::DismissCause::CloseButton)
    }

    pub(crate) fn batch1_release_history_event(
        &mut self,
        event: enzui_widgets::modal_host::ModalHostEvent,
    ) -> Task<Message> {
        self.reduce_release_history_host(event)
    }

    pub(crate) fn batch1_open_url(&mut self, url: String) -> Task<Message> {
        // Store's URL policy has exactly one owner: `UrlLauncher::start`
        // runs the parse + scheme screen and returns a typed
        // `StartOutcome`. The reducer never parses the URI
        // separately — doing so let a downstream caller collapse a
        // typed `ParseFailure` to generic `Failed`.
        //
        // A late portal reply arrives on `Message::OpenUrlFinished`;
        // a request that never starts routes through the same
        // finish arm, so `last_url_error` has one authoritative
        // source per attempt.
        // Wayland parent handles are asynchronous, so the open resumes after export.
        if let Some(window) = self.window {
            self.parent_export_generation = self.parent_export_generation.wrapping_add(1);
            let generation = self.parent_export_generation;
            return store_export_parent_task(
                &self.export_owner.dispatcher(),
                generation,
                window,
                self.url_launcher.parent_cancel(),
                url_launch::PARENT_EXPORT_TIMEOUT,
            )
            .map(move |parent| Message::OpenUrlParentExported {
                url: url.clone(),
                parent,
            });
        }
        let outcome = match self.url_launcher.start(&url) {
            Ok(()) => {
                self.last_url_error = None;
                // One immediate pump so a synchronous mock
                // reply (deterministic tests) lands in the same
                // reducer round. The production D-Bus backend
                // parks the reply and wakes via
                // `open_uri_completion_signal`, so this pump
                // returns `None` there and the subscription
                // drives the eventual `OpenUrlPortalTick`.
                self.url_launcher.pump()
            }
            Err(url_launch::StartOutcome::Parse(failure)) => {
                // A parse/policy refusal is terminal but not a
                // portal outcome: surface it directly and skip
                // the completion dispatch (no wire request was
                // ever installed).
                self.last_url_error = Some(failure.text().to_owned());
                return Task::none();
            }
            Err(url_launch::StartOutcome::Portal(outcome)) => Some(outcome),
        };
        if let Some(outcome) = outcome {
            // Map through the same finish path a late reply
            // takes, so `last_url_error` reflects one typed
            // outcome per attempt. The `OpenUrlFinished`
            // dispatch is still emitted so a subscription-driven
            // pump path also lands there consistently.
            self.last_url_error = url_launch::outcome_text(&outcome);
            return Task::done(Message::OpenUrlFinished(outcome));
        }
        Task::none()
    }

    pub(crate) fn batch1_window_opened(&mut self, id: iced::window::Id) -> Task<Message> {
        if self.window.is_some_and(|current| current != id) {
            return Task::none();
        }
        self.window = Some(id);
        self.chrome.bind(id);
        Task::batch([
            self.chrome
                .update(enzui_widgets::app_shell::ShellWindowEvent::Observed(id))
                .map(Message::WindowState),
            self.shell_activity.bind_window(id),
        ])
    }

    pub(crate) fn batch1_window_closed(&mut self, id: iced::window::Id) -> Task<Message> {
        if self.window == Some(id) && self.closing_window.is_none() {
            self.begin_close(id, enzui_core::portal::CancelReason::ParentClosed)
        } else {
            Task::none()
        }
    }

    pub(crate) fn batch1_open_url_parent_exported(
        &mut self,
        url: String,
        parent: enzui_parent_lease::Handoff<
            enzui_core::blocking_task::BlockingTaskSettlement<enzui_wayland::ParentAdoption>,
        >,
    ) -> Task<Message> {
        let Some(window) = self.window else {
            // Taking the one-shot settlement releases any export after
            // its window closes.
            drop(parent.take());
            return Task::none();
        };
        let Some(settlement) = parent.take() else {
            return Task::none();
        };
        // Refuse a settlement from a superseded export before adopting
        // it, so a slow export never parents onto a newer request.
        if settlement.generation() != self.parent_export_generation {
            return Task::none();
        }
        let adoption = match settlement.into_result() {
            Ok(adoption) => adoption,
            Err(error) => {
                // A pool refusal (Busy, a stopped or panicked worker, or
                // a cancelled request) is not a proven absence of a
                // parent: surface a typed, visible line rather than
                // opening an unparented request that fabricates one.
                self.last_url_error = Some(url_launch::parent_export_refusal_text(error));
                return Task::none();
            }
        };
        let outcome = match adoption {
            enzui_wayland::ParentAdoption::Parented { export, surface } => {
                match self
                    .url_launcher
                    .start_parented(&url, window, export, surface, |candidate| {
                        Some(candidate) == self.window
                    }) {
                    Ok(()) => {
                        self.last_url_error = None;
                        self.url_launcher.pump()
                    }
                    Err(url_launch::StartOutcome::Parse(failure)) => {
                        self.last_url_error = Some(failure.text().to_owned());
                        return Task::none();
                    }
                    Err(url_launch::StartOutcome::Portal(outcome)) => Some(outcome),
                }
            }
            // A proven no-Wayland-parent adoption (not a Wayland window,
            // or the window was gone) opens an explicit unparented
            // request.
            _ => match self.url_launcher.start(&url) {
                Ok(()) => {
                    self.last_url_error = None;
                    self.url_launcher.pump()
                }
                Err(url_launch::StartOutcome::Parse(failure)) => {
                    self.last_url_error = Some(failure.text().to_owned());
                    return Task::none();
                }
                Err(url_launch::StartOutcome::Portal(outcome)) => Some(outcome),
            },
        };
        if let Some(outcome) = outcome {
            self.last_url_error = url_launch::outcome_text(&outcome);
            return Task::done(Message::OpenUrlFinished(outcome));
        }
        Task::none()
    }

    pub(crate) fn batch1_open_url_portal_tick(
        &mut self,
        subscription_generation: u64,
    ) -> Task<Message> {
        // Reject a stale queued wake from a superseded request
        // generation: iced tears down the old subscription on a
        // key change, but a wake already delivered to the
        // reducer queue could still land here after the new
        // request installed a fresh generation. Pumping the
        // current request off a stale wake would pull a reply
        // meant for the old one.
        if subscription_generation != self.url_launcher.generation() {
            return Task::none();
        }
        if let Some(outcome) = self.url_launcher.pump() {
            return Task::done(Message::OpenUrlFinished(outcome));
        }
        Task::none()
    }

    pub(crate) fn batch1_open_url_finished(
        &mut self,
        outcome: url_launch::UrlLaunchOutcome,
    ) -> Task<Message> {
        // Every typed [`PortalOutcome`] arm has a distinct
        // visible line (or is silent when the outcome is a
        // decision, not a failure). `outcome_text` is exhaustive
        // — a new framework arm becomes a compile error there,
        // never a silently generic "failed".
        self.last_url_error = url_launch::outcome_text(&outcome);
        // A parent export must not outlive the portal request.
        self.url_launcher
            .release_parent(enzui_parent_lease::ReleaseCause::Settled);
        Task::none()
    }
}
