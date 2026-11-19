//! Release entries and history dialog.

use super::*;
use iced::widget::column;

pub(crate) fn release_entry<'a>(release: &crate::services::FlathubRelease) -> Element<'a, Message> {
    let version = if release.version.trim().is_empty() {
        crate::i18n::label("Unversioned").to_owned()
    } else {
        release.version.clone()
    };
    let date = release.formatted_date();
    let header = if date.is_empty() {
        version
    } else {
        format!("{version} · {date}")
    };
    let mut col = column![
        enz_text(header)
            .size(TextSize::Body)
            .weight(iced::font::Weight::Semibold)
            .color_resolved(|t| crate::theme::palette(t).text),
    ]
    .spacing(SpaceToken::Xxs.px())
    .width(Length::Fill);
    let notes = release.plain_description();
    let notes = notes.trim();
    if !notes.is_empty() {
        col = col.push(
            enz_text(notes.to_owned())
                .size(TextSize::Footnote)
                .color_resolved(|t| crate::theme::palette(t).text_muted),
        );
    }
    col.into()
}

/// Present newest-first changelogs in a capped dismissible modal, or preserve the background when closed.
pub(crate) fn present_release_history<'a>(
    store: &Store,
    window: iced::Size,
    background: Element<'a, Message>,
) -> Element<'a, Message> {
    if !store.release_history_open() {
        return crate::ui::closed_dialog_host(background);
    }
    let releases = store.detail_releases();
    if releases.is_empty() {
        return crate::ui::closed_dialog_host(background);
    }

    let theme: &EnzTheme = store.theme_ref();
    let sp = &theme.spacing;

    let title = container(
        enz_window_title(crate::i18n::label("Changelogs").to_owned()).tokens(WindowTitleTokens {
            title_size: 17.0,
            subtitle_size: 11.0,
            ..WindowTitleTokens::defaults()
        }),
    )
    .width(Length::Fill);

    let mut list = column![].spacing(SpaceToken::Sm.px()).width(Length::Fill);
    for (i, release) in releases.iter().enumerate() {
        list = list.push(release_entry(release));
        if i + 1 < releases.len() {
            list = list.push(chrome::hairline());
        }
    }

    let body = enz_scroll_view(container(list).padding(sp.sm()))
        .axis(Axis::Vertical)
        .height(Length::Shrink)
        .width(Length::Fill);

    let framed_body = enz_surface(body)
        .tone(SurfaceTone::SurfaceAlt)
        .border_style(BorderStyle::Thin)
        .border_width(1.0)
        .radius(6.0)
        .fill_width()
        .into_element();

    let actions = row![
        Space::new().width(Length::Fill),
        enz_button(crate::i18n::label("Close"), Message::CloseReleaseHistory)
            .kind(ButtonKind::Quiet),
    ]
    .spacing(sp.sm())
    .align_y(Alignment::Center);

    let stack = CappedDialogBody::new(
        vec![title.into()],
        framed_body,
        vec![actions.into()],
        12.0,
        window.height * DIALOG_HEIGHT_FRACTION,
    );

    let surface = enz_modal_surface(stack.into())
        .padding(sp.lg())
        .width(Length::Fixed(dialog_width(window.width)))
        .tokens(ModalSurfaceTokens::from_palette(&theme.palette))
        .into_element();

    enz_modal_host(background, Some(surface), Message::ReleaseHistoryEvent)
        .scoped_to(
            store
                .release_history_focus
                .as_ref()
                .expect("open release history has no focus lifecycle"),
        )
        .label(crate::i18n::label("Release history"))
        .into_element()
}
