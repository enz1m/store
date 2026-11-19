//! Library rows: installed rows, operation rows, errors, and
//! repository rows.

use super::RowDensity;
use crate::model::OperationState;
use crate::services::{AppEntry, RemoteInfo};
use crate::ui::chrome::{self, ChipTone};
use crate::{Message, Store};
use enzui_theme::spacing::SpaceToken;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_widgets::toolbar_icon::enz_toolbar_icon;
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Padding};

const ROW_PROGRESS_PILL_WIDTH: f32 = 116.0;

/// Resolve Update from its reporting installation independently of the installed copy used by Remove; no reported update yields no control.
pub(crate) fn installed_row<'a>(
    entry: &AppEntry,
    density: RowDensity,
    update: Option<crate::services::Authorization>,
    running: Option<&OperationState>,
) -> Element<'a, Message> {
    // Whether this row offers its own Update control. When it does, the
    // control (under the "Available updates" heading) already says so, and
    // the "Update available" chip only crowded the metadata strip until it
    // elided; it stays for an update this build cannot apply.
    let update_offered =
        entry.update_available && update.is_some_and(crate::services::Authorization::is_allowed);
    // Shared subtitle fallback prevents an empty band when every metadata chip is absent.
    let subtitle = chrome::AppRowSubtitle::<Message>::new(entry.id.clone())
        .push_chip_if(!entry.version.is_empty(), || {
            chrome::icon_chip("tag", entry.version.clone(), ChipTone::Muted)
        })
        .push_chip_if(!entry.size.is_empty(), || {
            chrome::icon_chip("hard-drive", entry.size.clone(), ChipTone::Muted)
        })
        .push_chip_if(density.shows_origin() && !entry.origin.is_empty(), || {
            chrome::icon_chip("server", entry.origin.clone(), ChipTone::Muted)
        })
        .push_chip_if(
            entry.update_available && !update_offered && density.shows_update_chip(),
            || {
                chrome::icon_chip(
                    "arrow-down-to-line",
                    crate::i18n::label("Update available"),
                    ChipTone::Accent,
                )
            },
        );

    let id_for_update = entry.id.clone();
    let name_for_update = entry.name.clone();
    let id_for_remove = entry.id.clone();
    let name_for_remove = entry.name.clone();
    let detail_message = Message::open_app_detail(entry);
    let row_detail_message = detail_message.clone();

    // Offered controls and reducer mutations must use the same authoritative installation decision.
    let mutation = entry.install_scope.mutation();

    let mut actions = row![]
        .spacing(SpaceToken::Space3.px())
        .align_y(Alignment::Center);

    actions = actions.push(
        enz_toolbar_icon("info", detail_message)
            .aria_label(crate::i18n::format(
                crate::i18n::localizer(),
                "Details for {}",
                &[("arg0", &(entry.name))],
            ))
            .tooltip(crate::i18n::label("Details")),
    );
    // A running update or removal turns the row's actions into its progress:
    // the pill where the Update button sits, and Cancel beside it.
    if let Some(op) = running {
        let progress = if op.cancel.cancel_is_actionable() || !op.cancel.offers_cancel() {
            chrome::progress_pill(
                op.kind.verb(),
                &op.name,
                op.completion(),
                Length::Fixed(ROW_PROGRESS_PILL_WIDTH),
                ButtonSize::Sm.tokens().height,
            )
        } else {
            // Reaping: the cancelled attempt's message replaces the meter.
            chrome::muted_line(op.message.clone())
        };
        actions = actions.push(progress);
        if let Some(cancel) = chrome::cancel_icon(op) {
            actions = actions.push(cancel);
        }
        return chrome::app_row(
            entry.icon_image.as_ref(),
            &entry.name,
            subtitle.build(),
            actions.into(),
            row_detail_message,
            None,
        );
    }
    // The "Update available" subtitle chip above is published for every scope
    // — a pending system update must not be invisible just because this build
    // cannot apply it — but the button that would apply it is only offered
    // where it can succeed.
    if update_offered {
        actions = actions.push(if !density.shows_action_labels() {
            enz_toolbar_icon(
                "arrow-down-to-line",
                Message::UpdateRequested(id_for_update, name_for_update),
            )
            .aria_label(crate::i18n::format(
                crate::i18n::localizer(),
                "Update {}",
                &[("arg0", &(entry.name))],
            ))
            .tooltip(crate::i18n::label("Update"))
            .into()
        } else {
            Element::from(
                enz_button(
                    crate::i18n::label("Update"),
                    Message::UpdateRequested(id_for_update, name_for_update),
                )
                .kind(ButtonKind::Accent)
                .size(ButtonSize::Sm)
                // The accessible name must not change with the window width.
                // The narrow rung above already says "Update <app>", because a
                // glyph-only button has to; this rung published the bare
                // visible text "Update" instead, so the same control answered
                // to two different names depending on how wide the window
                // happened to be — and in a list of installed apps, "Update"
                // alone does not say which app. The visible copy stays
                // "Update": the label is short because the row is tight, and
                // the row's own name supplies the context a sighted user has.
                .aria_label(crate::i18n::format(
                    crate::i18n::localizer(),
                    "Update {}",
                    &[("arg0", &(entry.name))],
                )),
            )
        });
    } else if entry.update_available
        && let Some(refusal) = update.and_then(crate::services::Authorization::refusal)
    {
        // The update is real and stays visible — hiding it would be the
        // original "the system scope was never read" defect again — but this
        // build cannot apply it. The row says which, in the update's own
        // vocabulary; the Remove disclosure below is a separate answer to a
        // separate question and may differ.
        actions = actions.push(refusal_chip(refusal));
    }
    match mutation.refusal() {
        None => {
            // Disclose any surviving installation before allowing removal of just this copy.
            if entry.system_copy_remains() {
                actions = actions.push(notice_chip(crate::services::SYSTEM_COPY_REMAINS));
            }
            // Remove is a *confirm*, never a direct uninstall — the
            // destructive-op guard from round one's plan.md, kept.
            actions = actions.push(
                enz_toolbar_icon(
                    "trash-2",
                    Message::UninstallConfirmRequested(id_for_remove, name_for_remove),
                )
                .aria_label(crate::i18n::format(
                    crate::i18n::localizer(),
                    "Remove {}",
                    &[("arg0", &(entry.name))],
                ))
                .tooltip(crate::i18n::label("Remove")),
            );
        }
        Some(refusal) => actions = actions.push(refusal_chip(refusal)),
    }

    chrome::app_row(
        entry.icon_image.as_ref(),
        &entry.name,
        subtitle.build(),
        actions.into(),
        row_detail_message,
        None,
    )
}

// The same [`chrome::app_row`] and [`chrome::operation_slot`] the installed
// rows and the other routes use, so one list is not rendered by two grammars.
// A failure keeps its row with the "Failed" chip; the failure itself is a
// bottom-right error toast (`Store::reconcile_failure_toasts`).
pub(super) fn operation_row<'a>(
    op: &OperationState,
    icon: Option<&crate::bounded_image::RenderImage>,
) -> Element<'a, Message> {
    // A live operation reports in the subtitle so the name keeps the row's
    // flexible width; the trailing slot holds only Cancel. A settled one keeps
    // its past-tense chip.
    let (subtitle, trailing): (Element<'a, Message>, Element<'a, Message>) = if op.finished {
        (
            chrome::muted_line(op.app_id.clone()),
            chrome::operation_slot(op, Length::Shrink, true),
        )
    } else {
        // The same pill and Cancel an installed app's row shows.
        let progress = if op.cancel.cancel_is_actionable() || !op.cancel.offers_cancel() {
            chrome::progress_pill(
                op.kind.verb(),
                &op.name,
                op.completion(),
                Length::Fixed(ROW_PROGRESS_PILL_WIDTH),
                ButtonSize::Sm.tokens().height,
            )
        } else {
            // Reaping: the cancelled attempt's message replaces the meter.
            chrome::muted_line(op.message.clone())
        };
        let mut trailing = row![progress]
            .spacing(SpaceToken::Space3.px())
            .align_y(Alignment::Center);
        if let Some(cancel) = chrome::cancel_icon(op) {
            trailing = trailing.push(cancel);
        }
        (chrome::muted_line(op.app_id.clone()), trailing.into())
    };

    chrome::app_row(
        icon,
        &op.name,
        subtitle,
        trailing,
        Message::OpenAppDetail(op.app_id.clone()),
        None,
    )
}

/// Report installed-read failure only after loading settles and without replacing a populated list with a stale error.
pub(super) fn read_failure(store: &Store) -> Option<String> {
    if store.loading_installed || !store.installed.is_empty() {
        return None;
    }
    store.last_error.clone()
}

/// The title of the installed-read-failed page. Not "no apps": a failure the
/// user can act on. Mirrors [`crate::ui::discover::catalog_error_title`].
pub(super) fn installed_error_title() -> &'static str {
    crate::i18n::label("Couldn’t read your installed apps")
}

/// The body of the installed-read-failed page, carrying the underlying failure
/// so the user is not told to retry a missing-`flatpak` error forever.
/// Mirrors [`crate::ui::discover::catalog_error_description`].
pub(super) fn installed_error_description(error: &str) -> String {
    format!(
        "flatpak could not be asked what is installed, so this list is not \
         empty — it is unknown. Check that flatpak is installed and try \
         again. ({error})"
    )
}

/// Describe failed repository reads separately from a successfully empty configuration.
pub(crate) fn repositories_error_title() -> &'static str {
    crate::i18n::label("Couldn’t read your repositories")
}

/// The body of the repositories-read-failed page, carrying flatpak's own
/// diagnostic so Retry is not offered forever against an unfixable cause.
/// Mirrors [`installed_error_description`].
pub(crate) fn repositories_error_description(error: &str) -> String {
    format!(
        "flatpak could not be asked which repositories are configured, so this \
         list is not empty — it is unknown. Until it is known, apps cannot be \
         installed, because Store cannot tell which repository would supply \
         them. ({error})"
    )
}

// A remote's display title, falling back to its name.
pub(crate) fn remote_title(remote: &RemoteInfo) -> String {
    if remote.title.is_empty() {
        remote.name.clone()
    } else {
        remote.title.clone()
    }
}

// Show installation scope so same-named repositories remain distinguishable.
pub(crate) fn remote_scope(remote: &RemoteInfo) -> Option<&'static str> {
    use crate::services::InstallScope;
    match InstallScope::from_remote_options(&remote.options) {
        InstallScope::User => Some(crate::i18n::label("User")),
        InstallScope::System => Some(crate::i18n::label("System")),
        // A remotes `options` cell never names an installation, so `Named` is
        // unreachable here; both non-answers keep the caller's "Unknown scope".
        InstallScope::Named(_) | InstallScope::Unknown => None,
    }
}

// Repository and app controls share the same mutation-authority decision.
pub(crate) fn remote_is_removable(remote: &RemoteInfo) -> bool {
    !crate::services::InstallScope::from_remote_options(&remote.options).is_system_managed()
}

// Keep refusal labels at every density: they provide the disclosure’s accessible name.
pub(crate) fn refusal_chip<'a>(refusal: crate::services::Refusal) -> Element<'a, Message> {
    chrome::icon_chip("lock", refusal.label, ChipTone::Muted)
}

// Keep partial-removal warnings named at every density; another installation may remain.
pub(crate) fn notice_chip<'a>(notice: crate::services::Notice) -> Element<'a, Message> {
    chrome::icon_chip("triangle-alert", notice.label, ChipTone::Warning)
}

pub(crate) const UPDATE_RETRY_LABEL: &str = "Retry update check";

pub(crate) const UPDATE_UNKNOWN_HEADING: &str = "Update check failed";

pub(crate) const UPDATE_PARTIAL_HEADING: &str = "Update check incomplete";

/// Disclose unknown update scopes, bounded failure reasons and partial-result limits, offering only actionable retries.
pub(crate) fn update_check_notice<'a>(store: &Store) -> Option<Element<'a, Message>> {
    let unknown = store.update_discovery.unknown_scopes();
    if unknown.is_empty() {
        return None;
    }
    let partial = store.updates_available().len();
    let heading = if partial == 0 {
        crate::i18n::label(UPDATE_UNKNOWN_HEADING)
    } else {
        UPDATE_PARTIAL_HEADING
    };
    // The chip primitive directly, not `notice_chip`: a `services::Notice`
    // pairs a chip label with the sentence a confirm dialog shows before an
    // action, and this band qualifies no action and precedes no dialog.
    // Borrowing the type with an empty `reason` would make it mean two things.
    let mut lines = column![
        row![
            chrome::icon_chip("triangle-alert", heading, ChipTone::Warning),
            Space::new().width(Length::Fill),
        ]
        .align_y(Alignment::Center)
    ]
    .spacing(SpaceToken::Xxs.px());

    for report in &unknown {
        let scope = report.scope.label();
        let reason = report
            .failure()
            .map(crate::services::UpdateScanFailure::disclosure)
            .unwrap_or_default();
        lines = lines.push(chrome::muted_caption(format!("{scope}: {reason}")));
    }
    if partial > 0 {
        // Stated explicitly rather than implied by the heading: a user
        // scanning the list needs to know the number they can see is a floor.
        lines = lines.push(chrome::muted_caption(format!(
            "{partial} pending update{} found in the installations that \
             answered; there may be more.",
            if partial == 1 { "" } else { "s" }
        )));
    } else {
        lines = lines.push(chrome::muted_caption(
            "No pending updates could be determined. This is not the same as \
             being up to date.",
        ));
    }
    if store.update_discovery.offers_retry() {
        let in_flight = store.update_check_in_flight();
        lines = lines.push(
            enz_button(
                if in_flight {
                    crate::i18n::label("Checking…")
                } else {
                    crate::i18n::label(UPDATE_RETRY_LABEL)
                },
                Message::UpdateCheckRetryRequested,
            )
            .kind(ButtonKind::Quiet)
            .size(ButtonSize::Sm)
            .icon("refresh-cw")
            // App-specific: this page also has a Refresh in the titlebar and a
            // per-row Update, so a bare "Retry" would not say what it retries.
            .aria_label(crate::i18n::label(UPDATE_RETRY_LABEL))
            // A retry already running must not be startable again; the label
            // changes too, so the state is visible and not only announced.
            .enabled(!in_flight),
        );
    }
    Some(
        container(lines)
            .padding(
                Padding::new(0.0)
                    .left(SpaceToken::Xs.px())
                    .top(SpaceToken::Xs.px())
                    .bottom(SpaceToken::Xs.px()),
            )
            .width(Length::Fill)
            .into(),
    )
}
