//! Shell trait bodies: subscription and view.

use super::*;

impl Store {
    pub(crate) fn build_subscription(&self) -> Subscription<Message> {
        // Escape navigates back only from Detail; top-level routes remain unchanged.
        let keys = if self.route == Route::Detail {
            iced::event::listen_with(classify_event_on_detail)
        } else {
            iced::event::listen_with(classify_event_off_detail)
        };
        // The publisher-link open wakes on the PORTAL's own completion
        // edge — one wake per portal event, gated on a live request. An
        // app with no chip armed holds no wake source at all, and a
        // settled reducer never runs a clock for a link opener. The
        // subscription payload carries the LAUNCHER request generation
        // that was live at subscription time; the tick arm rejects any
        // stale queued wake against `url_launcher.generation()`.
        let open_url_wake = self
            .url_launcher
            .completion_subscription()
            .map(Message::OpenUrlPortalTick);
        let window_identity = Subscription::batch([
            iced::window::open_events().map(Message::WindowOpened),
            iced::window::close_events().map(Message::WindowClosed),
        ]);
        let mut subscriptions = vec![
            self.instance.subscription(|_| Message::Launched),
            keys,
            self.themes
                .subscription()
                .map(|event| Message::Theme(Box::new(event))),
            open_url_wake,
            window_identity,
            self.chrome.subscription().map(Message::WindowState),
        ];
        if let Some(deadline) = self.activity_schedule.armed_deadline() {
            let remaining = deadline.saturating_sub(self.activity_now_ms()).max(1);
            subscriptions.push(
                iced::time::every(std::time::Duration::from_millis(remaining))
                    .with(deadline)
                    .map(|(deadline, _)| Message::ActivityPrune(deadline)),
            );
        }
        if !self.closing && self.deferred_owned_reads.needs_wake() {
            subscriptions.push(
                iced::time::every(READ_OWNER_RETRY_INTERVAL).map(|_| Message::ReadOwnerRetryTick),
            );
        }
        // Pending-only: armed while a hero intent waits for retry and a live
        // window owns it; it stops once the queue drains or the window closes.
        if self.window.is_some() && !self.featured_deferred.is_empty() {
            subscriptions.push(
                iced::time::every(FEATURED_RETRY_INTERVAL).map(|_| Message::FeaturedRetryTick),
            );
        }
        if !self.operation_completions.is_empty() {
            subscriptions
                .push(iced::time::every(OPERATION_POLL_INTERVAL).map(|_| Message::OperationPoll));
        }
        if self.closing {
            subscriptions.push(
                iced::time::every(std::time::Duration::from_millis(100))
                    .map(|_| Message::CloseDrainPoll),
            );
        }
        // One deadline-keyed wake while a toast can expire; nothing when idle.
        if let Some((deadline, wait)) = self.toast_expiry.armed() {
            subscriptions.push(
                iced::time::every(wait).with(deadline).map(|(_, now)| {
                    Message::Toast(enzui_widgets::toast_host::ToastEvent::Tick(now))
                }),
            );
        }
        #[cfg(feature = "center")]
        subscriptions.push(self.center_subscription());
        Subscription::batch(subscriptions)
    }

    pub(crate) fn render_page(&self) -> Element<'_, Message> {
        // The responsive closure is cached per distinct width per frame.
        enz_responsive(move |size| {
            let width = size.width;

            let page: Element<Message> = ui::shell::view(self, width);
            // The Repositories dialog sits right over the page, under the
            // Add repository dialog and the removal confirmation it opens.
            let page = ui::repositories::present(self, size, page);
            // The review runs in the background; only a failed review needs the dialog.
            let page = if self
                .install_consent
                .as_ref()
                .is_some_and(|consent| consent.error.is_some())
            {
                crate::install_consent::present(self, size, page)
            } else {
                ui::closed_dialog_host(page)
            };

            // The uninstall confirm is hosted over the whole page so Escape
            // and a scrim press resolve through the same funnel as the
            // buttons — the "one exit, not two" contract.
            let page: Element<Message> = match &self.pending_uninstall {
                Some((id, name)) => enz_confirm_dialog(
                    crate::i18n::format(
                        crate::i18n::localizer(),
                        "Remove {name}?",
                        &[("name", &(name))],
                    ),
                    // Confirm surviving system installations before removing the user copy.
                    if self.system_copy_survives_removal(id) {
                        services::SYSTEM_COPY_REMAINS.reason
                    } else {
                        crate::i18n::label(
                            "Its settings and data in your home folder are kept, so \
                             reinstalling it brings them back.",
                        )
                    },
                    &self.confirm_state,
                    Message::UninstallConfirmEvent,
                )
                .confirm_label(crate::i18n::label("Remove"))
                .cancel_label(crate::i18n::label("Keep"))
                .destructive(true)
                .id_prefix("store-uninstall")
                .into_host(page),
                None => ui::closed_dialog_host(page),
            };

            // Repository deletion is a second destructive action, not a direct
            // trash-icon side effect. Re-adding requires the repository URL,
            // so it receives its own confirmation and event funnel.
            let page: Element<Message> = match &self.pending_remote_removal {
                Some(name) => enz_confirm_dialog(
                    crate::i18n::format(
                        crate::i18n::localizer(),
                        "Remove {name} repository?",
                        &[("name", &(name))],
                    ),
                    "Installed apps stay installed, but this repository will no \
                 longer provide apps or updates.",
                    &self.remote_confirm_state,
                    Message::RemoveRemoteConfirmEvent,
                )
                .confirm_label(crate::i18n::label("Remove repository"))
                .cancel_label(crate::i18n::label("Keep repository"))
                .destructive(true)
                .id_prefix("store-remove-remote")
                .into_host(page),
                None => ui::closed_dialog_host(page),
            };

            let page = ui::detail::present(self, size, page);

            // Cluster A: the "Changelogs" release-history dialog, hosted over
            // the whole page like the permissions dialog. A no-op unless
            // `release_history_open` is set.
            let page = ui::detail::present_release_history(self, size, page);

            // Cluster B: the "Sandbox Permissions" editing dialog, hosted over
            // the whole page. A no-op unless `sandbox_dialog_open` is set.
            let page = ui::detail::present_sandbox_permissions(self, size, page);

            let page = ui::detail::present_screenshot_viewer(self, size, page);

            // Cluster 1: the add-remote form, hosted over the whole page so
            // Escape / scrim / Tab route through the same modal-host funnel. A
            // no-op unless `add_remote_form` is `Some`.
            // The close-recovery and close failures are bottom-right
            // notifications (`notification_view`), not banners above the page.
            ui::remotes::present(self, size, page)
        })
        .into()
    }

    /// The bottom-right stack: the "Store is still open" notice (its Retry
    /// and Keep Open stay until the close settles or it is dismissed) above
    /// the transient toasts.
    pub(crate) fn notification_view(&self) -> Option<Element<'_, Message>> {
        use enzui_widgets::notice_stack::{EnzNotice, enz_notice_stack};
        use enzui_widgets::toast::ToastTone;
        let notice = self
            .close_failure_text()
            .filter(|error| self.close_failure_dismissed.as_ref() != Some(error))
            .map(|error| {
                let (title, body) = ui::chrome::failure_copy(
                    crate::i18n::label("Store is still open"),
                    Some(error),
                );
                let mut notice = EnzNotice::new("store-close-failure", title, ToastTone::Error)
                    .action(crate::i18n::label("Retry"), Message::CloseRetry)
                    .secondary_action(crate::i18n::label("Keep Open"), Message::CloseKeepOpen)
                    .on_dismiss(Message::DismissCloseFailure);
                if let Some(body) = body {
                    notice = notice.description(body);
                }
                enz_notice_stack(vec![notice]).into_element()
            });
        let toasts = (!self.toasts.is_empty()).then(|| self.toasts.view().map(Message::Toast));
        match (notice, toasts) {
            (None, toasts) => toasts,
            (Some(notice), toasts) => Some(
                iced::widget::column(std::iter::once(notice).chain(toasts))
                    .spacing(self.theme_ref().spacing.xs())
                    .align_x(iced::Alignment::End)
                    .into(),
            ),
        }
    }
}
