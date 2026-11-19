use crate::{Message, STORE_ACTIVITY_PROVIDER, Store};
use enzui_activity::{CenterJobs, Publisher, Visibility};
use enzui_core::operation_activity::ActivityKind;
use iced::{Task, window};

#[derive(Default)]
pub(crate) struct ShellActivity {
    publisher: Option<Publisher>,
    jobs: CenterJobs,
    window: Option<window::Id>,
    pending: Option<String>,
    token: Option<String>,
}

impl ShellActivity {
    #[cfg(not(test))]
    pub(crate) fn production() -> Self {
        Self {
            publisher: Publisher::new("io.github.enzui.Store").ok(),
            ..Self::default()
        }
    }

    pub(crate) fn bind_window(&mut self, window: window::Id) -> Task<Message> {
        if self.publisher.is_none() || self.window == Some(window) {
            return Task::none();
        }
        self.window = Some(window);
        self.token = None;
        self.pending = enzui_activity::new_window_token().ok();
        let Some(tag) = self.pending.clone() else {
            return Task::none();
        };
        iced::window::set_toplevel_tag(window, tag.clone())
            .map(move |queued| Message::ActivityWindowTagged(window, tag.clone(), queued))
    }

    pub(crate) fn confirm_tag(&mut self, window: window::Id, tag: String, queued: bool) {
        if self.window == Some(window) && self.pending.as_deref() == Some(&tag) {
            self.token = queued.then_some(tag);
            self.pending = None;
        }
    }
}

impl Store {
    pub(crate) fn sync_shell_activity(&mut self) {
        let Some(publisher) = &self.shell_activity.publisher else {
            return;
        };
        self.shell_activity.jobs.sync(
            publisher,
            &self.activity_center,
            self.shell_activity.token.as_deref(),
            Visibility::WhenUnfocused,
            |row| {
                if row.key.provider != STORE_ACTIVITY_PROVIDER {
                    return None;
                }
                match row.kind {
                    ActivityKind::Other(verb) => Some(verb.to_owned()),
                    _ => None,
                }
            },
        );
    }
}
