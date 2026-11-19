//! Permissions present and dialog width.

use super::*;

/// Present the permissions modal with capped content and framework dismissal, or preserve the background when closed.
pub(crate) fn present<'a>(
    store: &Store,
    window: iced::Size,
    background: Element<'a, Message>,
) -> Element<'a, Message> {
    if !store.permissions_dialog_open() {
        return crate::ui::closed_dialog_host(background);
    }

    let theme: &EnzTheme = store.theme_ref();
    let sp = &theme.spacing;

    let app_id = store.detail_app_id_ref().unwrap_or("");
    let name = store
        .app_display_name(app_id)
        .unwrap_or_else(|| app_id.to_owned());

    let title_tokens = WindowTitleTokens {
        title_size: 17.0,
        subtitle_size: 11.0,
        ..WindowTitleTokens::defaults()
    };
    let title = container(
        enz_window_title(crate::i18n::label("Permissions").to_owned())
            .subtitle(name)
            .tokens(title_tokens),
    )
    .width(Length::Fill);

    let body: Element<'a, Message> = enz_scroll_view(
        container(super::permission_list::permission_list(
            store.detail_permissions(),
        ))
        .padding(iced::Padding::new(0.0).right(sp.sm())),
    )
    .axis(Axis::Vertical)
    .height(Length::Shrink)
    .width(Length::Fill)
    .into();

    let actions = row![
        Space::new().width(Length::Fill),
        enz_button(crate::i18n::label("Close"), Message::ClosePermissionsDialog)
            .kind(ButtonKind::Quiet),
    ]
    .spacing(sp.sm())
    .align_y(Alignment::Center);

    let stack = CappedDialogBody::new(
        vec![title.into()],
        body,
        vec![actions.into()],
        12.0,
        window.height * DIALOG_HEIGHT_FRACTION,
    );

    let surface = enz_modal_surface(stack.into())
        .padding(sp.lg())
        .width(Length::Fixed(dialog_width(window.width)))
        .tokens(ModalSurfaceTokens::from_palette(&theme.palette))
        .into_element();

    // The WHOLE modal-host event is routed through `PermissionsDialogEvent`
    // (Escape / scrim / Tab), not `|_| Close`: a Tab press must wrap focus
    // inside the dialog, and a genuine dismissal must both close it and restore
    // focus to the invoking button.
    enz_modal_host(background, Some(surface), Message::PermissionsDialogEvent)
        .scoped_to(
            store
                .permissions_focus
                .as_ref()
                .expect("open permissions dialog has no focus lifecycle"),
        )
        .label(crate::i18n::label("App permissions"))
        .into_element()
}

/// The dialog's width for a window width — capped so it does not sprawl on a
/// wide monitor, and never wider than the window minus a margin. Identical
/// policy to `task-manager-app::view::logs::dialog_width`.
pub(crate) fn dialog_width(window_width: f32) -> f32 {
    (window_width - 48.0).clamp(240.0, 640.0)
}
