//! The **Repositories** dialog, opened from the titlebar.
//!
//! Where Store finds apps, one compact row per repository, with the daily
//! automatic-updates switch under them. Adding a repository opens the Add
//! repository dialog over this one; removing one asks first.

use enzui_theme::SpaceToken;
use enzui_widgets::capped_dialog_body::CappedDialogBody;
use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_widgets::modal_host::enz_modal_host;
use enzui_widgets::modal_surface::{ModalSurfaceTokens, enz_modal_surface};
use enzui_widgets::surface::{BorderStyle, SurfaceTone, enz_surface};
use enzui_widgets::window_title::{WindowTitleTokens, enz_window_title};
use enzui_widgets::{ButtonKind, enz_button};
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length};

use crate::services::RemoteInfo;
use crate::ui::chrome;
use crate::{Message, Store};

/// The focus scope of the dialog.
pub(crate) const REPOSITORIES_SCOPE: &str = "store-repositories-dialog";
/// The titlebar button that opens it, where focus returns on close.
pub(crate) const REPOSITORIES_INVOKER_ID: &str = "store-manage-repositories";

const DIALOG_MAX_WIDTH: f32 = 560.0;

/// Present the dialog over `background` while it is open.
pub(crate) fn present<'a>(
    store: &'a Store,
    window: iced::Size,
    background: Element<'a, Message>,
) -> Element<'a, Message> {
    let Some(focus) = store.repositories_focus.as_ref() else {
        return crate::ui::closed_dialog_host(background);
    };
    let theme = store.theme_ref();
    let title = container(
        enz_window_title(crate::i18n::label("Repositories").to_owned())
            .subtitle(crate::i18n::label("Where Store finds apps").to_owned())
            .tokens(WindowTitleTokens {
                title_size: 17.0,
                subtitle_size: 11.0,
                ..WindowTitleTokens::defaults()
            }),
    )
    .width(Length::Fill);

    // An incomplete update check is reported here, by the repositories it
    // concerns, rather than on the Library page.
    let mut body = column![].spacing(SpaceToken::Md.px()).width(Length::Fill);
    if let Some(notice) = crate::ui::library::update_check_notice(store) {
        body = body.push(notice);
    }
    let mut body = body.push(repository_list(store));
    // The switch only means something where enzimOS's updater reads it.
    if store.automatic_updater {
        body = body.push(automatic_updates(store));
    }

    let actions = row![
        enz_button(
            crate::i18n::label("Add repository"),
            Message::AddRemoteOpened
        )
        .kind(ButtonKind::Quiet)
        .icon("plus"),
        Space::new().width(Length::Fill),
        enz_button(
            crate::i18n::label("Close"),
            Message::CloseRepositoriesDialog
        )
        .kind(ButtonKind::Quiet),
    ]
    .spacing(SpaceToken::Sm.px())
    .align_y(Alignment::Center);

    let stack = CappedDialogBody::new(
        vec![title.into()],
        {
            let scroll: Element<'a, Message> = enzui_widgets::scroll_view::enz_scroll_view(body)
                .axis(enzui_widgets::scroll_view::Axis::Vertical)
                .height(Length::Shrink)
                .width(Length::Fill)
                .into();
            scroll
        },
        vec![actions.into()],
        12.0,
        window.height * 0.8,
    );
    let surface = enz_modal_surface(stack.into())
        .padding(SpaceToken::Lg.px())
        .width(Length::Fixed(
            (window.width - 48.0).clamp(280.0, DIALOG_MAX_WIDTH),
        ))
        .tokens(ModalSurfaceTokens::from_palette(&theme.palette))
        .into_element();
    enz_modal_host(background, Some(surface), Message::RepositoriesDialogEvent)
        .scoped_to(focus)
        .label(crate::i18n::label("Repositories"))
        .into_element()
}

/// The repositories in one framed list, or why there are none to show.
fn repository_list(store: &Store) -> Element<'_, Message> {
    if let Some(error) = store.repositories_error() {
        return column![
            chrome::muted_line(crate::ui::library::repositories_error_title()),
            chrome::muted_caption(crate::ui::library::repositories_error_description(error)),
            enz_button(
                crate::i18n::label(crate::ui::discover::RETRY_LABEL),
                Message::RefreshRequested,
            )
            .kind(ButtonKind::Accent)
            .icon("refresh-cw")
            .aria_label(crate::i18n::label("Retry reading repositories")),
        ]
        .spacing(SpaceToken::Sm.px())
        .into();
    }
    if store.remotes_load == crate::services::RemoteLoad::Loading {
        return chrome::muted_line(crate::services::REPOSITORIES_LOADING.label);
    }
    if store.remotes.is_empty() {
        return chrome::muted_line(crate::i18n::label("No repositories configured"));
    }
    let mut list = column![].width(Length::Fill);
    for (index, remote) in store.remotes.iter().enumerate() {
        if index > 0 {
            list = list.push(chrome::hairline());
        }
        list = list.push(repository_row(remote, update_failure(store, remote)));
    }
    enz_surface(list)
        .tone(SurfaceTone::SurfaceAlt)
        .border_style(BorderStyle::Thin)
        .border_width(1.0)
        .radius(10.0)
        .fill_width()
        .into_element()
}

/// One repository: its name with its installation, the URL under it, and
/// Remove where Store may remove it.
fn repository_row<'a>(remote: &RemoteInfo, failure: Option<String>) -> Element<'a, Message> {
    let name = remote.name.clone();
    let title = crate::ui::library::remote_title(remote);
    let scope =
        crate::ui::library::remote_scope(remote).unwrap_or(crate::i18n::label("Unknown scope"));
    let url = if remote.url.is_empty() {
        remote.options.clone()
    } else {
        remote.url.clone()
    };
    let heading = row![
        enz_text(title)
            .size(TextSize::Body)
            .weight(iced::font::Weight::Semibold)
            .single_line_elided(),
        enz_text(scope.to_owned())
            .size(TextSize::Caption)
            .tone(TextTone::Muted),
    ]
    .spacing(SpaceToken::Sm.px())
    .align_y(Alignment::Center);
    let mut text = column![
        heading,
        enz_text(url)
            .size(TextSize::Footnote)
            .tone(TextTone::Muted)
            .single_line_elided()
            .full_width(),
    ]
    .spacing(2.0)
    .width(Length::Fill);
    if let Some(failure) = failure {
        text = text.push(chrome::icon_chip(
            "triangle-alert",
            failure,
            chrome::ChipTone::Warning,
        ));
    }

    let trailing: Element<'a, Message> = if crate::ui::library::remote_is_removable(remote) {
        enzui_widgets::toolbar_icon::enz_toolbar_icon(
            "trash-2",
            Message::RemoveRemoteRequested(name.clone()),
        )
        .label(crate::i18n::format(
            crate::i18n::localizer(),
            "Remove repository {name}",
            &[("name", &(name))],
        ))
        .into()
    } else {
        // A system repository belongs to the distribution: it can be
        // neither removed nor edited here.
        chrome::icon_chip(
            "lock",
            crate::services::SYSTEM_MANAGED.label,
            chrome::ChipTone::Muted,
        )
    };
    row![text, trailing]
        .spacing(SpaceToken::Md.px())
        .align_y(Alignment::Center)
        .padding([SpaceToken::Sm.px(), SpaceToken::Md.px()])
        .into()
}

/// Why this repository is missing from the last update check, when it did
/// not answer it.
fn update_failure(store: &Store, remote: &RemoteInfo) -> Option<String> {
    let scope = crate::services::InstallScope::from_remote_options(&remote.options);
    store
        .update_discovery
        .unknown_scopes()
        .into_iter()
        .find_map(|report| match report.failure() {
            Some(
                failure @ crate::services::UpdateScanFailure::Timeout {
                    remote: Some(name), ..
                },
            ) if *name == remote.name && report.scope == scope => {
                let sentence = failure.disclosure();
                let mut chars = sentence.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
            }
            _ => None,
        })
}

/// The daily background updates switch.
fn automatic_updates(store: &Store) -> Element<'_, Message> {
    let mut text = column![
        enz_text(crate::i18n::label("Update apps automatically").to_owned()).size(TextSize::Body),
        enz_text(crate::i18n::label(crate::ui::library::AUTOMATIC_UPDATES_DESCRIPTION).to_owned())
            .size(TextSize::Footnote)
            .tone(TextTone::Muted),
    ]
    .spacing(2.0)
    .width(Length::Fill);
    if let Some(error) = &store.automatic_updates_error {
        text = text.push(chrome::muted_caption(error.clone()));
    }
    let toggle: Element<'_, Message> =
        enzui_widgets::switch::switch(store.automatic_updates, Message::AutomaticUpdatesToggled)
            .label(crate::i18n::label("Update apps automatically"))
            .into();
    row![text, toggle]
        .spacing(SpaceToken::Md.px())
        .align_y(Alignment::Center)
        .padding([0.0, SpaceToken::Xs.px()])
        .into()
}
