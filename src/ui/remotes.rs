//! Add and replace repository forms retain the repository list behind their modal.

use enzui_core::semantics::HeadingLevel;
use enzui_theme::EnzTheme;
use enzui_theme::SpaceToken;
use enzui_widgets::capped_dialog_body::CappedDialogBody;
use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_widgets::modal_host::enz_modal_host;
use enzui_widgets::modal_surface::{ModalSurfaceTokens, enz_modal_surface};
use enzui_widgets::text_field::enz_text_field;
use enzui_widgets::{ButtonKind, enz_button, enz_form, form_field, form_row};
use iced::widget::{Space, container, row};
use iced::{Alignment, Element, Length};

use crate::{Message, Store};

/// The modal-host scope key for the add-remote dialog. Focus operations
/// (initial focus, Tab-wrapping) are resolved against this at traversal time,
/// so a dialog that has since closed declines structurally.
pub(crate) const ADD_REMOTE_SCOPE: &str = "store-add-remote";

/// The name field's routed id — the focus target "Rename" hands back to the
/// user and the field itself share this exact identity.
pub(crate) fn add_remote_name_field_id() -> iced::advanced::widget::Id {
    enzui_core::identity::derive_id(NAME_FIELD_PREFIX, crate::i18n::label("Name"))
}

/// The URL field's routed id, defined beside the name field's for the same
/// reason: one definition per routed id, not one per use site.
pub(crate) fn add_remote_url_field_id() -> iced::advanced::widget::Id {
    enzui_core::identity::derive_id(URL_FIELD_PREFIX, "URL")
}

/// Id-prefix for the name field, so the reducer's focus operations and the
/// rendered field derive one identity.
pub(crate) const NAME_FIELD_PREFIX: &str = "store-add-remote-name";
/// Id-prefix for the URL field.
pub(crate) const URL_FIELD_PREFIX: &str = "store-add-remote-url";

/// The dialog's width for a window width — capped so it does not sprawl on a
/// wide monitor and never wider than the window minus a margin. Same policy as
/// [`crate::ui::detail::dialog_width`].
pub(crate) fn dialog_width(window_width: f32) -> f32 {
    (window_width - 48.0).clamp(240.0, 560.0)
}

/// Present Add Repository with one modal dismissal and focus boundary, or preserve the background when closed.
pub(crate) fn present<'a>(
    store: &'a Store,
    window: iced::Size,
    background: Element<'a, Message>,
) -> Element<'a, Message> {
    let Some(form) = store.add_remote_form_ref() else {
        return crate::ui::closed_dialog_host(background);
    };

    let theme: &EnzTheme = store.theme_ref();
    let sp = &theme.spacing;

    let title = container(
        enz_text(crate::i18n::label("Add repository"))
            .size(TextSize::Title2)
            .tone(TextTone::Default)
            .heading(HeadingLevel::Page),
    )
    .width(Length::Fill);

    // Pin the routed ids because the visible captions now belong to the form rows.
    let name_field = enz_text_field("", "flathub", &form.name, Message::AddRemoteNameChanged)
        .id(add_remote_name_field_id())
        .aria_label(crate::i18n::text("Remote name"))
        .into_element();

    let url_field = enz_text_field(
        "",
        "https://dl.flathub.org/repo/flathub.flatpakrepo",
        &form.url,
        Message::AddRemoteUrlChanged,
    )
    .id(add_remote_url_field_id())
    .aria_label(crate::i18n::text("Repository URL"))
    .on_submit(Message::AddRemoteSubmit)
    .into_element();

    let mut rows = vec![
        form_field(crate::i18n::label("Name"), name_field),
        form_field("URL", url_field),
    ];

    // A retained name conflict is an unattempted choice, not a mutation failure.
    if let Some(conflict) = form.conflict.as_ref() {
        rows.push(form_row(
            super::chrome::failure_banner(conflict.message(), None).into_element(),
        ));
    }
    if let Some(err) = form.error.as_deref() {
        rows.push(form_row(
            super::chrome::failure_banner(err.to_owned(), None).into_element(),
        ));
    }

    // The modal surface owns the outer padding; a form inset would double it.
    let body: Element<'a, Message> = enz_form(rows)
        .inset(SpaceToken::None)
        .row_gap(SpaceToken::Md)
        .spacing_scale(*sp)
        .width(Length::Fill)
        .into_element();

    // Conflicts require an explicit resolution; in-flight reads and mutations disable every competing action.
    let busy = form.busy;
    let mut actions = row![Space::new().width(Length::Fill)]
        .spacing(sp.sm())
        .align_y(Alignment::Center);
    actions = actions.push(
        enz_button(crate::i18n::label("Cancel"), Message::AddRemoteCancelled)
            .kind(ButtonKind::Quiet)
            .enabled(!busy),
    );
    match form.conflict.as_ref() {
        Some(conflict) => {
            actions = actions
                .push(
                    enz_button(
                        crate::i18n::label("Rename"),
                        Message::AddRemoteRenameRequested,
                    )
                    .kind(ButtonKind::Quiet)
                    .enabled(!busy)
                    .aria_label(crate::i18n::format(
                        crate::i18n::localizer(),
                        "Rename instead of replacing {}",
                        &[("arg0", &(conflict.name))],
                    )),
                )
                .push(
                    enz_button(
                        crate::i18n::label("Replace"),
                        Message::AddRemoteReplaceConfirmed,
                    )
                    .kind(ButtonKind::Accent)
                    .enabled(!busy)
                    // Accessible conflict labels mask the same credentials as visual disclosures.
                    .aria_label(crate::i18n::format(
                        crate::i18n::localizer(),
                        "Replace {} with {}",
                        &[
                            ("arg0", &(conflict.shown_existing_url())),
                            ("arg1", &(conflict.shown_requested_url())),
                        ],
                    )),
                );
        }
        None => {
            actions = actions.push(
                enz_button(
                    crate::i18n::label("Add repository"),
                    Message::AddRemoteSubmit,
                )
                .kind(ButtonKind::Accent)
                .enabled(!busy),
            );
        }
    }

    // Capped so the surface's action row is a child of the surface column, not
    // an overlay past a mis-measured box — the "half-floating button" defect.
    let stack = CappedDialogBody::new(
        vec![title.into()],
        body,
        vec![actions.into()],
        12.0,
        window.height * 0.8,
    );

    let surface = enz_modal_surface(stack.into())
        .padding(sp.lg())
        .width(Length::Fixed(dialog_width(window.width)))
        .tokens(ModalSurfaceTokens::from_palette(&theme.palette))
        .into_element();

    enz_modal_host(background, Some(surface), Message::AddRemoteDialogEvent)
        .scoped_to(
            store
                .add_remote_focus
                .as_ref()
                .expect("open Add Remote dialog has no focus lifecycle"),
        )
        .label(crate::i18n::label("Add repository"))
        .into_element()
}
