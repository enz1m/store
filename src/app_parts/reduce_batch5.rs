//! Fifth reducer batch: remote removal, theme, window close.

use super::*;

use iced::Task;

impl Store {
    pub(crate) fn batch5_add_remote_dialog_event(
        &mut self,
        event: enzui_widgets::modal_host::ModalHostEvent,
    ) -> Task<Message> {
        self.reduce_add_remote_host(event)
    }

    pub(crate) fn batch5_remote_add_planned(
        &mut self,
        generation: u64,
        result: Result<services::RemoteAddPlan, String>,
    ) -> Task<Message> {
        // Ignore results for abandoned drafts rather than applying them to a reopened dialog.
        if !self.add_remote_generation_is_current(generation) {
            return Task::none();
        }
        if let Some(form) = self.add_remote_form.as_mut() {
            form.busy = false;
        }
        let plan = match result {
            Ok(plan) => plan,
            Err(error) => return self.fail_add_remote(error),
        };
        match plan {
            services::RemoteAddPlan::Add => {
                let Some(form) = self.add_remote_form.as_ref() else {
                    return Task::none();
                };
                let name = form.name.trim().to_owned();
                let url = form.url.trim().to_owned();
                if let Some(form) = self.add_remote_form.as_mut() {
                    form.busy = true;
                }
                let task = self
                    .read_owner
                    .submit(generation, move |_ctx| services::add_remote(&name, &url));
                read_pool_task(
                    task,
                    move |result| Message::RemoteAdded(generation, result),
                    move |_error| {
                        Message::RemoteAdded(generation, Err(READ_POOL_REMOTE_REFUSAL.to_owned()))
                    },
                )
            }
            // The requested state already holds. This is the one
            // honest idempotent success, and it is reached WITHOUT
            // running a child process: the read that proved it is the
            // whole of the work.
            services::RemoteAddPlan::AlreadyConfigured => self.settle_add_remote(),
            // Retain the form. Nothing has been mutated and nothing
            // will be until the user picks Replace or renames.
            services::RemoteAddPlan::UrlConflict { existing_url } => {
                if let Some(form) = self.add_remote_form.as_mut() {
                    form.conflict = Some(model::RemoteConflict {
                        name: form.name.trim().to_owned(),
                        existing_url,
                        requested_url: form.url.trim().to_owned(),
                    });
                }
                Task::none()
            }
            // The name belongs to an installation this build cannot
            // write. Stated as the refusal's own reason, with the form
            // retained so Rename is one edit away.
            services::RemoteAddPlan::ScopeConflict(refusal) => {
                self.fail_add_remote(refusal.reason.to_owned())
            }
        }
    }

    pub(crate) fn batch5_add_remote_replace_confirmed(&mut self) -> Task<Message> {
        let Some(form) = self.add_remote_form.as_ref() else {
            return Task::none();
        };
        // Replace runs only from a retained conflict, so the argv is
        // built from the draft the read actually checked rather than
        // whatever the fields hold now.
        let Some(conflict) = form.conflict.clone() else {
            return Task::none();
        };
        if form.busy {
            return Task::none();
        }
        if let Some(form) = self.add_remote_form.as_mut() {
            form.busy = true;
            form.error = None;
        }
        let generation = self.add_remote_generation;
        let task = self.read_owner.submit(generation, move |_ctx| {
            services::replace_remote_url(
                &conflict.name,
                &conflict.requested_url,
                &conflict.existing_url,
            )
        });
        read_pool_task(
            task,
            move |result| Message::RemoteReplaced(generation, result),
            move |_error| {
                Message::RemoteReplaced(generation, Err(READ_POOL_REMOTE_REFUSAL.to_owned()))
            },
        )
    }

    pub(crate) fn batch5_add_remote_rename_requested(&mut self) -> Task<Message> {
        // Not a Flatpak call: the conflict is dropped and the name
        // field takes focus, which is the whole of "Rename".
        let Some(form) = self.add_remote_form.as_mut() else {
            return Task::none();
        };
        if form.busy {
            return Task::none();
        }
        form.conflict = None;
        form.error = None;
        operate(iced::advanced::widget::operation::focusable::focus(
            ui::remotes::add_remote_name_field_id(),
        ))
    }

    pub(crate) fn batch5_remote_replaced(
        &mut self,
        generation: u64,
        result: Result<String, String>,
    ) -> Task<Message> {
        if !self.add_remote_generation_is_current(generation) {
            return Task::none();
        }
        if let Some(form) = self.add_remote_form.as_mut() {
            form.busy = false;
        }
        match result {
            // The URL was read back off the repository, so this is a
            // confirmed state rather than an exit code.
            Ok(_settled) => self.settle_add_remote(),
            // The old repository is intact and the draft is retained,
            // conflict included, so Replace can be retried.
            Err(error) => self.fail_add_remote(error),
        }
    }

    pub(crate) fn batch5_remote_added(
        &mut self,
        generation: u64,
        result: Result<(), String>,
    ) -> Task<Message> {
        if !self.add_remote_generation_is_current(generation) {
            return Task::none();
        }
        if let Some(form) = self.add_remote_form.as_mut() {
            form.busy = false;
        }
        match result {
            Ok(()) => self.settle_add_remote(),
            Err(e) => self.fail_add_remote(e),
        }
    }

    pub(crate) fn batch5_remove_remote_requested(&mut self, name: String) -> Task<Message> {
        // Removing a repository changes where future installs and
        // updates can come from. Arm the same canonical destructive
        // gate as uninstall; do not run `remote-delete` on the first
        // trash-icon press.
        self.pending_remote_removal = Some(name);
        self.remote_confirm_state = enzui_core::confirm::ConfirmState::new(None);
        Task::none()
    }

    pub(crate) fn batch5_remove_remote_confirm_event(
        &mut self,
        event: ConfirmDialogEvent,
    ) -> Task<Message> {
        match event {
            ConfirmDialogEvent::Resolved(cause) => {
                let confirmed =
                    enzui_core::confirm::ConfirmDecision::from_cause(cause).is_confirmed();
                let pending = self.pending_remote_removal.take();
                match (confirmed, pending) {
                    (true, Some(name)) => {
                        let result_name = name.clone();
                        let refused_name = name.clone();
                        let task = self
                            .read_owner
                            .submit(0, move |_ctx| services::remove_remote(&name));
                        read_pool_task(
                            task,
                            move |result| Message::RemoteRemoved(result_name, result),
                            move |_error| {
                                Message::RemoteRemoved(
                                    refused_name,
                                    Err(READ_POOL_REMOTE_REFUSAL.to_owned()),
                                )
                            },
                        )
                    }
                    _ => Task::none(),
                }
            }
            ConfirmDialogEvent::FocusNext | ConfirmDialogEvent::FocusPrevious => {
                match event.focus_operation::<Message>(true) {
                    Some(op) => iced::advanced::widget::operate(op),
                    None => Task::none(),
                }
            }
            ConfirmDialogEvent::CheckboxToggled(_) => Task::none(),
        }
    }

    pub(crate) fn batch5_remote_removed(
        &mut self,
        name: String,
        result: Result<(), String>,
    ) -> Task<Message> {
        match result {
            Ok(()) => {
                self.last_error = None;
                // Reflect the successful destructive action immediately;
                // the backend refresh remains authoritative and follows.
                self.remotes.retain(|remote| remote.name != name);
                self.start_remotes_read()
            }
            Err(e) => {
                self.last_error = Some(e);
                Task::none()
            }
        }
    }

    pub(crate) fn batch5_theme(&mut self, event: Box<ThemeEvent>) -> Task<Message> {
        let before = self.themes.theme_generation();
        let task = self.themes.update(*event);
        // `SubtreeHandle` keys framework style generation and
        // invalidates itself; catalogue/projection/image caches are
        // content-derived and deliberately stay warm.
        debug_assert!(self.themes.theme_generation() >= before);
        if self.themes.theme_generation() != before {
            self.library_cache.invalidate();
        }
        task.map(|event| Message::Theme(Box::new(event)))
    }

    pub(crate) fn batch5_window_gesture(
        &mut self,
        command: enzui_widgets::title_bar_gesture::WindowCommand,
    ) -> Task<Message> {
        if command == enzui_widgets::title_bar_gesture::WindowCommand::Close {
            return self
                .window
                .map_or_else(Task::none, |id| Task::done(Message::CloseRequested(id)));
        }
        self.chrome.run_command(command)
    }

    pub(crate) fn batch5_close_requested(&mut self, id: iced::window::Id) -> Task<Message> {
        if self.window != Some(id) || self.closing_window.is_some() {
            return Task::none();
        }
        self.begin_close(id, enzui_core::portal::CancelReason::Application)
    }

    pub(crate) fn batch5_close_retry(&mut self) -> Task<Message> {
        self.retry_close_drain()
    }

    pub(crate) fn batch5_close_keep_open(&mut self) -> Task<Message> {
        self.close_keep_open_requested = true;
        self.poll_close_drain()
    }

    pub(crate) fn batch5_close_drain_poll(&mut self) -> Task<Message> {
        self.poll_close_drain()
    }
}
