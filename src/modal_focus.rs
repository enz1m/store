//! Owned focus for Store's Permissions and Add Remote hosts.
//!
//! The read-only **Permissions** dialog captures the "View permissions" button
//! as its invoker. **Add Remote** has no single tracked invoker (it is reached
//! from a Library button, an empty-state button and the topbar `+`), so it
//! restores nothing and instead carries a **generation**: `flatpak remote-add`
//! resolves off the reducer thread, and a completion minted by a form the user
//! already cancelled must not close or repopulate a reopened one.

use enzui_core::modal::DismissCause;
use enzui_widgets::modal_host::{ModalFocusLifecycle, ModalHostEvent};
use iced::Task;

use crate::model::AddRemoteForm;
use crate::{Message, Store, ui};

impl Store {
    pub(crate) fn open_release_history(&mut self) -> Task<Message> {
        if self.detail_releases().is_empty() {
            return Task::none();
        }
        let (focus, op) = ModalFocusLifecycle::open::<Message>(
            ui::detail::RELEASE_HISTORY_SCOPE_KEY,
            Some(ui::detail::release_history_invoker_id()),
        );
        self.release_history_open = true;
        self.release_history_focus = Some(focus);
        iced::advanced::widget::operate(op)
    }

    pub(crate) fn dismiss_release_history(&mut self, cause: DismissCause) -> Task<Message> {
        self.release_history_open = false;
        self.release_history_focus
            .take()
            .map_or_else(Task::none, |focus| {
                focus
                    .reduce::<Message>(ModalHostEvent::Dismissed(cause))
                    .into_task()
            })
    }

    pub(crate) fn reduce_release_history_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some(focus) = self.release_history_focus.as_ref() else {
            return Task::none();
        };
        let outcome = focus.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            self.release_history_open = false;
            self.release_history_focus = None;
        }
        outcome.into_task()
    }

    pub(crate) fn open_sandbox_dialog(&mut self) -> Task<Message> {
        let (focus, op) = ModalFocusLifecycle::open::<Message>(
            ui::detail::SANDBOX_SCOPE_KEY,
            Some(ui::detail::sandbox_invoker_id()),
        );
        self.sandbox_dialog_open = true;
        self.sandbox_dialog_focus = Some(focus);
        iced::advanced::widget::operate(op)
    }

    pub(crate) fn dismiss_sandbox_dialog(&mut self, cause: DismissCause) -> Task<Message> {
        self.sandbox_dialog_open = false;
        self.sandbox_dialog_focus
            .take()
            .map_or_else(Task::none, |focus| {
                focus
                    .reduce::<Message>(ModalHostEvent::Dismissed(cause))
                    .into_task()
            })
    }

    pub(crate) fn reduce_sandbox_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some(focus) = self.sandbox_dialog_focus.as_ref() else {
            return Task::none();
        };
        let outcome = focus.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            self.sandbox_dialog_open = false;
            self.sandbox_dialog_focus = None;
        }
        outcome.into_task()
    }

    pub(crate) fn close_detail_modals_for_host(&mut self) -> Vec<Task<Message>> {
        self.release_history_open = false;
        self.sandbox_dialog_open = false;
        [
            &mut self.release_history_focus,
            &mut self.sandbox_dialog_focus,
        ]
        .into_iter()
        .filter_map(Option::take)
        .map(|focus| focus.close_for_host::<Message>().into_task())
        .collect()
    }

    // Present the permissions dialog: capture the invoker and land focus on
    // the dialog's first control.
    pub(crate) fn open_permissions_dialog(&mut self) -> Task<Message> {
        let (lifecycle, open_op) = ModalFocusLifecycle::open::<Message>(
            ui::detail::PERMISSIONS_SCOPE_KEY,
            Some(ui::detail::permissions_invoker_id()),
        );
        self.permissions_dialog_open = true;
        self.permissions_focus = Some(lifecycle);
        iced::advanced::widget::operate(open_op)
    }

    // A logical permissions dismissal (the footer Close): restore the exact
    // invoker through the owner.
    pub(crate) fn dismiss_permissions_dialog(&mut self, cause: DismissCause) -> Task<Message> {
        if !self.permissions_dialog_open {
            return Task::none();
        }
        self.permissions_dialog_open = false;
        self.permissions_focus
            .take()
            .map_or_else(Task::none, |lifecycle| {
                lifecycle
                    .reduce::<Message>(ModalHostEvent::Dismissed(cause))
                    .into_task()
            })
    }

    // Reduce a permissions host event through its owner.
    pub(crate) fn reduce_permissions_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some(lifecycle) = self.permissions_focus.as_ref() else {
            return Task::none();
        };
        let outcome = lifecycle.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            self.permissions_dialog_open = false;
            self.permissions_focus = None;
        }
        outcome.into_task()
    }

    // The detail page navigated away: the permissions host is gone with it,
    // so its owner is retired without stealing focus from the new page.
    pub(crate) fn close_permissions_for_host(&mut self) -> Option<Task<Message>> {
        self.permissions_dialog_open = false;
        self.permissions_focus
            .take()
            .map(|lifecycle| lifecycle.close_for_host::<Message>().into_task())
    }

    // Present the Repositories dialog, returning focus to its titlebar button
    // when it closes.
    pub(crate) fn open_repositories_dialog(&mut self) -> Task<Message> {
        if self.repositories_focus.is_some() {
            return Task::none();
        }
        self.search_popup_open = false;
        let (lifecycle, open_op) = ModalFocusLifecycle::open::<Message>(
            ui::repositories::REPOSITORIES_SCOPE,
            Some(iced::advanced::widget::Id::new(
                ui::repositories::REPOSITORIES_INVOKER_ID,
            )),
        );
        self.repositories_focus = Some(lifecycle);
        iced::advanced::widget::operate(open_op)
    }

    pub(crate) fn dismiss_repositories_dialog(&mut self, cause: DismissCause) -> Task<Message> {
        self.repositories_focus
            .take()
            .map_or_else(Task::none, |lifecycle| {
                lifecycle
                    .reduce::<Message>(ModalHostEvent::Dismissed(cause))
                    .into_task()
            })
    }

    pub(crate) fn reduce_repositories_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        // A dialog over this one (Add repository, a removal confirmation)
        // owns Escape and the scrim while it is up.
        if self.add_remote_form.is_some() || self.pending_remote_removal.is_some() {
            return Task::none();
        }
        let Some(lifecycle) = self.repositories_focus.as_ref() else {
            return Task::none();
        };
        let outcome = lifecycle.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            self.repositories_focus = None;
        }
        outcome.into_task()
    }

    // Present a fresh Add Remote form on a new generation.
    pub(crate) fn open_add_remote(&mut self) -> Task<Message> {
        let close_old = self
            .add_remote_focus
            .take()
            .map(|lifecycle| lifecycle.close_for_host::<Message>().into_task());
        self.add_remote_generation = self.add_remote_generation.saturating_add(1);
        let (lifecycle, open_op) =
            ModalFocusLifecycle::open::<Message>(ui::remotes::ADD_REMOTE_SCOPE, None);
        self.add_remote_form = Some(AddRemoteForm::default());
        self.add_remote_focus = Some(lifecycle);
        let open = iced::advanced::widget::operate(open_op);
        match close_old {
            Some(close) => Task::batch([close, open]),
            None => open,
        }
    }

    // A logical Add Remote dismissal (Cancel, or a committed add).
    pub(crate) fn dismiss_add_remote(&mut self, cause: DismissCause) -> Task<Message> {
        self.add_remote_form = None;
        self.add_remote_focus
            .take()
            .map_or_else(Task::none, |lifecycle| {
                lifecycle
                    .reduce::<Message>(ModalHostEvent::Dismissed(cause))
                    .into_task()
            })
    }

    // Reduce an Add Remote host event through its owner.
    pub(crate) fn reduce_add_remote_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some(lifecycle) = self.add_remote_focus.as_ref() else {
            return Task::none();
        };
        let outcome = lifecycle.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            self.add_remote_form = None;
            self.add_remote_focus = None;
        }
        outcome.into_task()
    }

    // Every draft edit advances the generation; results may settle only against that same draft.
    pub(crate) fn add_remote_generation_is_current(&self, generation: u64) -> bool {
        generation == self.add_remote_generation && self.add_remote_form.is_some()
    }

    pub(crate) fn edit_add_remote_draft(&mut self, edit: impl FnOnce(&mut AddRemoteForm)) {
        let Some(form) = self.add_remote_form.as_mut() else {
            return;
        };
        edit(form);
        // A stale error or conflict described the *last* submitted draft, not
        // this one.
        form.error = None;
        form.conflict = None;
        // An in-flight read or mutation was started for the draft that just
        // stopped existing.
        form.busy = false;
        self.add_remote_generation = self.add_remote_generation.saturating_add(1);
    }

    // Successful add, replacement and no-op paths all refresh the authoritative repository list.
    pub(crate) fn settle_add_remote(&mut self) -> Task<Message> {
        self.last_error = None;
        let dismiss = self.dismiss_add_remote(DismissCause::DefaultAction);
        let remotes = self.start_remotes_read();
        Task::batch([dismiss, remotes])
    }

    pub(crate) fn fail_add_remote(&mut self, error: String) -> Task<Message> {
        if let Some(form) = self.add_remote_form.as_mut() {
            form.busy = false;
            form.error = Some(error.clone());
        }
        self.last_error = Some(error);
        Task::none()
    }
}
