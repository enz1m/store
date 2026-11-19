//! Sandbox permissions dialog.

use super::*;
use iced::widget::column;

/// Edit effective sandbox permissions while disclosing each declared default and supporting reset to defaults.
pub(crate) fn present_sandbox_permissions<'a>(
    store: &'a Store,
    window: iced::Size,
    background: Element<'a, Message>,
) -> Element<'a, Message> {
    if !store.sandbox_dialog_open() {
        return crate::ui::closed_dialog_host(background);
    }

    let theme: &EnzTheme = store.theme_ref();
    let sp = &theme.spacing;

    let title = container(
        enz_window_title(crate::i18n::label("Sandbox Permissions").to_owned()).tokens(
            WindowTitleTokens {
                title_size: 17.0,
                subtitle_size: 11.0,
                ..WindowTitleTokens::defaults()
            },
        ),
    )
    .width(Length::Fill);

    let app_id = store.detail_app_id_ref().unwrap_or("");
    let policy_copy = if store.is_installed(app_id) {
        crate::i18n::label(
            "Changes update this installed app's user policy and take effect on next launch.",
        )
    } else {
        crate::i18n::label("This is a user policy that will apply when the app is installed.")
    };
    let mut rows = column![].spacing(SpaceToken::Sm.px()).width(Length::Fill);

    if store.permission_toggles_unavailable() {
        // A typed unavailable model is never rendered as an editable
        // all-disabled set: state it plainly and offer a retry, with every
        // mutation control disabled below.
        rows = rows.push(chrome::muted_line(crate::i18n::label(
            "Sandbox permissions are unavailable.",
        )));
        rows = rows.push(
            enz_button(
                crate::i18n::label("Retry"),
                Message::SandboxPermissionsRetry,
            )
            .kind(ButtonKind::Ghost)
            .size(ButtonSize::Sm)
            .icon("refresh-cw")
            .aria_label(crate::i18n::label("Retry reading sandbox permissions")),
        );
    } else if store.permission_toggles().is_empty() {
        // COMMON_TOGGLES is never empty once Ready, so an empty model is the
        // build still in flight.
        rows = rows.push(chrome::muted_line("Reading permissions\u{2026}"));
    } else {
        for t in store.permission_toggles() {
            let key = t.key.clone();
            // Label effective policy rather than misnaming overrides as app defaults.
            let subtitle =
                crate::model::PermissionDefault::from_toggle(t.app_default, t.enabled).label();
            let icon = match t.category.as_str() {
                "network" => "wifi",
                "filesystem" => "folder",
                "socket" => "monitor",
                "device" => "usb",
                _ => "shield",
            };
            let row = enzui_widgets::preferences_row::enz_preferences_row(t.label.as_str())
                .subtitle(subtitle)
                .icon(icon)
                .switch(t.enabled, move |on| {
                    Message::TogglePermission(key.clone(), on)
                })
                .into_element();
            rows = rows.push(row);
        }
    }

    if let Some(snapshot) = &store.shared_permission_snapshot {
        rows = rows.push(chrome::muted_line(crate::i18n::label("Flatpak context includes inherited user policy. Broad access can bypass portal prompts; runtime conditions may affect access.")));
        rows = rows.push(enzui_widgets::disclosure::enz_disclosure(
            crate::i18n::label("Full access and policy sources"),
            store.shared_permission_details,
            Message::SandboxPolicyDetails,
        ));
        if store.shared_permission_details {
            if let Some(groups) = snapshot["flatpak_computed_context"].as_object() {
                for (group, values) in groups {
                    rows = rows.push(enz_text(group.clone()).size(TextSize::Headline));
                    if let Some(values) = values.as_object() {
                        for (key, value) in values {
                            rows = rows.push(chrome::muted_line(format!(
                                "{}: {}",
                                key,
                                value
                                    .as_str()
                                    .unwrap_or(crate::i18n::label("Unknown value"))
                            )));
                        }
                    }
                }
            }
            if let Some(sources) = snapshot["sources"].as_array() {
                for source in sources {
                    let scope = source["scope"]
                        .as_str()
                        .unwrap_or(crate::i18n::label("Unknown"));
                    let target = if source["target"] == "global" {
                        crate::i18n::label("All apps")
                    } else {
                        crate::i18n::label("This app")
                    };
                    let applies = source["applicable"].as_bool().unwrap_or(false);
                    rows = rows.push(
                        enz_text(format!(
                            "{scope} policy · {target}{}",
                            if applies {
                                ""
                            } else {
                                crate::i18n::label(" · does not apply")
                            }
                        ))
                        .size(TextSize::Headline),
                    );
                    if applies && let Some(groups) = source["policy"].as_object() {
                        for (group, values) in groups {
                            if let Some(values) = values.as_object() {
                                for (key, value) in values {
                                    rows = rows.push(chrome::muted_line(format!(
                                        "{group} · {key}: {}",
                                        value
                                            .as_str()
                                            .unwrap_or(crate::i18n::label("Unknown value"))
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(error) = store.permission_error() {
        rows = rows.push(
            enz_text(error.to_owned())
                .size(TextSize::Footnote)
                .color_resolved(|t| crate::theme::palette(t).danger),
        );
    }

    let body = enz_scroll_view(container(rows).padding(sp.sm()))
        .axis(Axis::Vertical)
        .height(Length::Shrink)
        .width(Length::Fill);

    let framed_body = enzui_widgets::preferences_group::enz_preferences_group(body)
        .aria_label(crate::i18n::label("Sandbox permission policy"))
        .description(policy_copy)
        .bordered(true)
        .width(Length::Fill)
        .into_element();

    let actions = row![
        enz_button(
            crate::i18n::label("Reset to Defaults"),
            Message::ResetPermissionsToDefaults
        )
        .kind(ButtonKind::Quiet)
        .size(ButtonSize::Sm)
        .icon("rotate-ccw")
        // Mutation stays disabled unless the editable model is authoritative.
        .enabled(store.permission_toggles_ready()),
        Space::new().width(Length::Fill),
        enz_button(crate::i18n::label("Close"), Message::CloseSandboxDialog)
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

    enz_modal_host(background, Some(surface), Message::SandboxDialogEvent)
        .scoped_to(
            store
                .sandbox_dialog_focus
                .as_ref()
                .expect("open sandbox dialog has no focus lifecycle"),
        )
        .label(crate::i18n::label("Sandbox permissions"))
        .into_element()
}
