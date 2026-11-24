//! Invokers and the detail page view.

use super::*;
use iced::widget::column;

pub(crate) fn release_history_invoker_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("store-see-all-changelogs")
}

pub(crate) fn sandbox_invoker_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("store-edit-sandbox-permissions")
}

pub(crate) fn permissions_invoker_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new(PERMISSIONS_INVOKER_ID)
}

/// Render app identity, lifecycle actions and responsive metadata groups, omitting unavailable metadata.
pub(crate) fn view<'a>(store: &'a Store, app_id: &str, width: f32) -> Element<'a, Message> {
    let sp = &store.theme_ref().spacing;
    let entry = store.app_entry(app_id);
    let name = entry
        .as_ref()
        .map_or_else(|| app_id.to_owned(), |e| e.name.clone());

    // ---- header: icon + (name / publisher · id) --------------------------
    let icon = entry.as_ref().and_then(|e| e.icon_image.clone());

    let title = enz_text(name.clone())
        .size(TextSize::Title2)
        .weight(iced::font::Weight::Semibold)
        .color_resolved(|theme| crate::theme::palette(theme).text)
        .heading(HeadingLevel::Page);

    // The publisher, when the catalogue names one; the app id otherwise. The
    // id is always in the information grid below.
    let byline = match entry.as_ref().map(|e| e.publisher.trim().to_owned()) {
        Some(pubr) if !pubr.is_empty() => pubr,
        _ => app_id.to_owned(),
    };
    let subtitle = enz_text(byline)
        .size(TextSize::Body)
        .single_line_elided()
        .color_resolved(|theme| crate::theme::palette(theme).text_muted);

    let identity = column![
        title,
        subtitle,
        publisher_verification(store, app_id, &name)
    ]
    .spacing(SpaceToken::Xs.px());

    // Header actions sit on the identity's right while the row has room for
    // both, and move under it on a phone-width window.
    let stacked = width < DETAIL_HEADER_STACK_WIDTH;
    let controls = header_actions(store, app_id, &name, !stacked);
    let header: Element<'a, Message> = if stacked {
        column![
            row![
                chrome::app_icon(icon.as_ref(), &name, HEADER_ICON_PX),
                identity
            ]
            .spacing(SpaceToken::Lg.px())
            .align_y(Alignment::Center),
            controls,
        ]
        .spacing(SpaceToken::Md.px())
        .width(Length::Fill)
        .into()
    } else {
        row![
            chrome::app_icon(icon.as_ref(), &name, HEADER_ICON_PX),
            identity.width(Length::Fill),
            controls,
        ]
        .spacing(SpaceToken::Lg.px())
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
    };

    let mut page = column![header].spacing(SpaceToken::Lg.px());
    if let Some(error) = store.detail_request_error() {
        page = page.push(
            chrome::failure_banner(
                crate::i18n::label("Details unavailable"),
                Some(error.to_owned()),
            )
            .into_element(),
        );
    }
    // An operation failure is a bottom-right error toast
    // (`Store::reconcile_failure_toasts`), not a banner on the page.

    // ---- summary ---------------------------------------------------------
    if let Some(e) = entry.as_ref()
        && !e.summary.trim().is_empty()
    {
        page = page.push(
            enz_text(e.summary.clone())
                .size(TextSize::Body)
                .color_resolved(|theme| crate::theme::palette(theme).text),
        );
    }

    // ---- description -----------------------------------------------------
    if let Some(e) = entry.as_ref() {
        let desc = e.description.trim();
        if !desc.is_empty() && desc != e.summary.trim() {
            let desc = desc.to_owned();
            page = page.push(
                enz_text(desc)
                    .size(TextSize::Body)
                    .color_resolved(|theme| crate::theme::palette(theme).text_muted),
            );
        }
    }

    if !store.screenshots().is_empty() {
        page = page.push(chrome::section_label(crate::i18n::label("Screenshots")));
        page = page.push(screenshot_gallery(store));
        if store.screenshot_load_state() == crate::ScreenshotLoadState::CachedStale {
            page = page.push(chrome::muted_line(crate::i18n::label(
                "Showing cached screenshots; no current replacement is available.",
            )));
        } else if store.screenshot_load_state() == crate::ScreenshotLoadState::Partial {
            page = page.push(chrome::muted_line(crate::i18n::label(
                "Some screenshots could not be loaded.",
            )));
        }
    } else if store.screenshot_load_state() == crate::ScreenshotLoadState::KnownEmpty {
        page = page.push(chrome::section_label(crate::i18n::label("Screenshots")));
        let empty: Element<'a, Message> = container(
            enzui_widgets::status_page::enz_status_page(crate::services::no_screenshots_label())
                .icon("image-off")
                .description(crate::services::no_screenshots_aria()),
        )
        .width(Length::Fill)
        .into();
        page = page.push(empty);
    } else if store.screenshot_load_state() == crate::ScreenshotLoadState::CachedStale {
        page = page.push(chrome::section_label(crate::i18n::label("Screenshots")));
        page = page.push(chrome::muted_line(crate::i18n::label(
            "Cached metadata has no screenshots; no current result is available.",
        )));
    } else if store.screenshot_load_state() == crate::ScreenshotLoadState::Failed {
        page = page.push(chrome::section_label(crate::i18n::label("Screenshots")));
        page = page.push(
            container(
                enzui_widgets::status_page::enz_status_page(crate::i18n::label(
                    "Screenshots unavailable",
                ))
                .icon("image-off")
                .description(crate::i18n::label(
                    "Store could not download a usable screenshot.",
                )),
            )
            .width(Length::Fill),
        );
    } else {
        // Most apps have screenshots: hold the gallery's place while they load.
        page = page.push(chrome::section_label(crate::i18n::label("Screenshots")));
        page = page.push(screenshot_gallery_placeholder());
    }

    // ---- About this app ---------------------------------------------------
    // An information grid that flows into as many columns as the width holds,
    // then the publisher's links as one row of chips.
    page = page.push(
        column![
            section_header(crate::i18n::label("About this app"), None),
            about_section(store, entry.as_ref(), app_id),
        ]
        .spacing(SECTION_HEADER_GAP),
    );

    if !store.detail_from_enzimos(app_id) {
        page = page.push(
            column![
                section_header(crate::i18n::label("Ratings"), None),
                two_column(
                    detail_card(user_ratings_block(store)),
                    detail_card(content_rating_block(store, entry.is_some())),
                    width,
                ),
            ]
            .spacing(SECTION_HEADER_GAP),
        );
    }

    let releases = store.detail_releases();
    let history: Option<Element<'a, Message>> = (releases.len() > 1).then(|| {
        enz_button(
            crate::model::see_all_changelogs_label(),
            Message::OpenReleaseHistory,
        )
        .kind(ButtonKind::Secondary)
        .size(ButtonSize::Sm)
        .icon("history")
        .id(release_history_invoker_id())
        .into()
    });
    let mut whats_new_body = column![].spacing(SpaceToken::Sm.px()).width(Length::Fill);
    if let Some(latest) = releases.first() {
        whats_new_body = whats_new_body.push(release_entry(latest));
    } else if matches!(
        store
            .detail_context()
            .map(|context| &context.metadata_state),
        Some(crate::services::DetailRouteState::Failed)
    ) {
        whats_new_body = whats_new_body.push(chrome::muted_line(crate::i18n::label(
            "Release notes unavailable.",
        )));
    } else if store.detail_context().is_none()
        || matches!(
            store
                .detail_context()
                .map(|context| &context.metadata_state),
            Some(crate::services::DetailRouteState::Loading)
        )
    {
        // The latest release's heading and the first line of its notes.
        let label = crate::i18n::label("Loading release notes…");
        whats_new_body = whats_new_body.push(
            column![
                chrome::skeleton_block(Length::Fixed(200.0), 18.0, label),
                chrome::skeleton_block(Length::Fill, 14.0, label),
            ]
            .spacing(SpaceToken::Sm.px()),
        );
    } else {
        whats_new_body = whats_new_body.push(chrome::muted_line(crate::i18n::label(
            "No release notes available.",
        )));
    }
    if matches!(
        store
            .detail_context()
            .map(|context| &context.metadata_state),
        Some(crate::services::DetailRouteState::CachedStale { .. })
    ) {
        whats_new_body = whats_new_body.push(chrome::muted_line(crate::i18n::label(
            "Showing cached release notes while refreshing.",
        )));
    }
    let whats_new_block = column![
        section_header(whats_new_heading(), history),
        detail_card(whats_new_body.into()),
    ]
    .spacing(SECTION_HEADER_GAP);
    let whats_new_block: Element<'a, Message> = whats_new_block.width(Length::Fill).into();

    // The single permissions invoker owns focus restoration; declared and editable policies remain separate surfaces.
    // The control that opens the read-only dialog. Carries
    // `PERMISSIONS_INVOKER_ID` so focus returns here on dismissal.
    let open = enz_button(
        crate::i18n::label("View permissions"),
        Message::OpenPermissionsDialog,
    )
    .kind(ButtonKind::Secondary)
    .size(ButtonSize::Sm)
    .icon("shield")
    .icon_position(enzui_widgets::button::ButtonIconPosition::Leading)
    .id(permissions_invoker_id());
    // The editing entry point — opens the "Sandbox Permissions" dialog.
    let manage = enz_button(
        crate::i18n::label("Sandbox Permissions"),
        Message::OpenSandboxDialog,
    )
    .kind(ButtonKind::Secondary)
    .size(ButtonSize::Sm)
    .icon("sliders-horizontal")
    .icon_position(enzui_widgets::button::ButtonIconPosition::Leading)
    .id(sandbox_invoker_id());

    let mut permissions_body = column![
        chrome::muted_line(permission_glance(store.detail_permissions())),
        permission_pills(store.detail_permissions()),
    ]
    .spacing(SpaceToken::Sm.px())
    .width(Length::Fill);
    // A failed permission read is typed and retryable: surface the action only
    // on a real failure, never while the read is still in flight, and never in
    // place of a declared set. The muted line above already reads
    // "unavailable" rather than a reassuring "no exceptions" claim.
    // Editing an app's access goes through the enzimOS permission service;
    // elsewhere only the read-only view is offered.
    let mut actions = row![open]
        .spacing(SpaceToken::Sm.px())
        .align_y(Alignment::Center);
    if store.permission_service {
        actions = actions.push(manage);
    }
    if store.detail_permissions_unavailable() {
        actions = actions.push(
            enz_button(
                crate::i18n::label("Retry"),
                Message::PermissionsRetryRequested(app_id.to_owned()),
            )
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .icon("refresh-cw")
            .icon_position(enzui_widgets::button::ButtonIconPosition::Leading)
            // App-specific: a bare "Retry" on a page that can also retry
            // verification or screenshots does not say what is being retried.
            .aria_label(crate::i18n::format(
                crate::i18n::localizer(),
                "Retry reading permissions for {name}",
                &[("name", &(name))],
            )),
        );
    }
    permissions_body = permissions_body
        .push(container(actions.wrap()).padding(iced::Padding::new(0.0).top(SpaceToken::Xs.px())));
    let permissions_block = column![
        section_header(crate::i18n::label("Permissions & Sandbox"), None),
        detail_card(permissions_body.into()),
    ]
    .spacing(SECTION_HEADER_GAP);

    page = page.push(whats_new_block);
    page = page.push(permissions_block);

    // Scrollable so a long description / large permission set never clips.
    // A wide window keeps the page at the readable measure the list routes
    // use instead of stretching a sentence across the whole pane.
    let scroll = enz_scroll_view(
        container(chrome::readable_measure(page.width(Length::Fill))).padding(sp.lg()),
    )
    .axis(Axis::Vertical)
    .id(iced::advanced::widget::Id::new(crate::DETAIL_SCROLL_ID))
    .width(Length::Fill)
    .height(Length::Fill);

    container(scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

const HEADER_ICON_PX: f32 = 64.0;

const DETAIL_HEADER_STACK_WIDTH: f32 = 600.0;

pub(crate) const PRIMARY_ACTION_MIN_WIDTH: f32 = 96.0;

const VERIFICATION_SKELETON_WIDTH: f32 = 120.0;
const VERIFICATION_SKELETON_HEIGHT: f32 = 13.0;

/// A titled section of the page (`Title3`, a section heading), with the
/// section's own action, when it has one, at the far end of the line.
fn section_header<'a>(
    label: &'static str,
    trailing: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let title = enz_text(label.to_owned())
        .size(TextSize::Title3)
        .weight(iced::font::Weight::Semibold)
        .color_resolved(|theme| crate::theme::palette(theme).text)
        .heading(HeadingLevel::Section);
    let mut line = row![container(title).width(Length::Fill)]
        .align_y(Alignment::Center)
        .width(Length::Fill);
    if let Some(trailing) = trailing {
        line = line.push(trailing);
    }
    // Every section starts the same distance below the one before it.
    container(line)
        .padding(iced::Padding::new(0.0).top(SpaceToken::Md.px()))
        .width(Length::Fill)
        .into()
}

/// The gap between a section's heading and its card.
const SECTION_HEADER_GAP: f32 = 8.0;

/// One of the page's cards: the raised list surface the dialogs use.
fn detail_card<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    enz_surface(
        container(content)
            .padding(SpaceToken::Lg.px())
            .width(Length::Fill),
    )
    .tone(SurfaceTone::SurfaceAlt)
    .border_style(BorderStyle::Thin)
    .border_width(1.0)
    .radius(10.0)
    .fill_width()
    .into_element()
}

/// The publisher-verification chip under the app's name, with its staleness
/// or failure disclosure (and Retry) when the answer is not current.
fn publisher_verification<'a>(store: &'a Store, app_id: &str, name: &str) -> Element<'a, Message> {
    if store.detail_from_enzimos(app_id) {
        return row![
            chrome::enzimos_source_chip(),
            chrome::icon_chip(
                "badge-check",
                crate::i18n::label("Repository verified"),
                chrome::ChipTone::Success,
            ),
        ]
        .spacing(SpaceToken::Sm.px())
        .align_y(Alignment::Center)
        .wrap()
        .into();
    }
    // Only the enrichment verdict establishes verification; absent context remains pending without Retry.
    let publisher_verification = store
        .detail_context()
        .map_or_else(crate::services::PublisherVerification::pending, |ctx| {
            ctx.verification.clone()
        });
    let verification_chip = chrome::icon_chip(
        crate::model::verification_icon(&publisher_verification),
        crate::model::verification_label(&publisher_verification),
        match &publisher_verification {
            crate::services::PublisherVerification::Verified { .. } => chrome::ChipTone::Success,
            crate::services::PublisherVerification::KnownUnverified { .. } => {
                chrome::ChipTone::Muted
            }
            // Warning, not Muted: muted is what a settled negative looks like,
            // and this is not settled.
            crate::services::PublisherVerification::Unknown(_) => chrome::ChipTone::Warning,
        },
    );
    // The chip alone cannot say WHY, and an unknown trust state that does not
    // say why is only marginally better than a wrong one. The disclosure is
    // bounded at the source (`VerificationFailure::disclosure`), because the
    // transport and status details are external text.
    match &publisher_verification {
        // Still asking (or superseded by a newer load): hold the chip's place
        // instead of flashing a warning that a moment later turns verified.
        crate::services::PublisherVerification::Unknown(unavailable)
            if matches!(
                unavailable.reason,
                crate::services::VerificationFailure::Pending
                    | crate::services::VerificationFailure::Cancelled
            ) =>
        {
            chrome::skeleton_block(
                Length::Fixed(VERIFICATION_SKELETON_WIDTH),
                VERIFICATION_SKELETON_HEIGHT,
                unavailable.reason.disclosure(),
            )
        }
        crate::services::PublisherVerification::Unknown(unavailable) => {
            let mut line = row![
                verification_chip,
                enz_text(unavailable.reason.disclosure())
                    .size(TextSize::Caption)
                    .tone(TextTone::Muted),
            ]
            .spacing(SpaceToken::Sm.px())
            .align_y(Alignment::Center);
            if unavailable.retry.offers_retry() {
                line = line.push(
                    enz_button(
                        crate::i18n::label("Retry"),
                        Message::VerificationRetryRequested(app_id.to_owned()),
                    )
                    .kind(ButtonKind::Ghost)
                    .size(ButtonSize::Xs)
                    .icon("refresh-cw")
                    // App-specific: a bare "Retry" in a page that can also
                    // retry screenshots does not say what is being retried.
                    .aria_label(crate::i18n::format(
                        crate::i18n::localizer(),
                        "Retry publisher verification for {name}",
                        &[("name", &(name))],
                    )),
                );
            }
            line.wrap().into()
        }
        // A known answer that was RETAINED after a later check failed states
        // its age beside the chip. An answer whose age cannot be stated never
        // reaches this branch: it is discarded into `Unknown` at the reducer.
        known => match known
            .provenance()
            .and_then(crate::services::VerificationProvenance::staleness)
        {
            Some(staleness) => row![
                verification_chip,
                enz_text(staleness)
                    .size(TextSize::Caption)
                    .tone(TextTone::Muted),
            ]
            .spacing(SpaceToken::Sm.px())
            .align_y(Alignment::Center)
            .wrap()
            .into(),
            None => verification_chip,
        },
    }
}

/// The header's action group: one filled primary action (Install, Open or
/// Update), the secondary actions beside it at the same height, and any
/// disclosure about what those actions cannot do on a line under them.
///
/// While an operation runs, its progress takes the primary's place, with
/// Cancel beside it where Remove sat.
fn header_actions<'a>(
    store: &'a Store,
    app_id: &str,
    name: &str,
    align_end: bool,
) -> Element<'a, Message> {
    let installed = store.is_installed(app_id);
    let has_update = store.has_update(app_id);
    // Remove uses installed scope, Update uses reporting scope, and Install uses source-repository authority.
    let mutation = store.action_authorization(app_id, crate::model::OpKind::Uninstall);
    let update_authorization = store.action_authorization(app_id, crate::model::OpKind::Update);
    let install_authorization = store.action_authorization(app_id, crate::model::OpKind::Install);

    let mut buttons: Vec<Element<'a, Message>> = Vec::new();
    let mut notes: Vec<Element<'a, Message>> = Vec::new();

    if let Some(op) = store
        .operation_for(app_id)
        .filter(|op| !op.finished && op.error.is_none())
    {
        buttons.push(chrome::operation_slot(
            op,
            Length::Fixed(DETAIL_ACTION_WIDTH),
            true,
        ));
        if op.cancel.offers_cancel() {
            buttons.push(
                enz_button(
                    enzui_core::operation_activity::ActivityControl::Cancel.label(),
                    Message::CancelOperationRequested(op.id()),
                )
                .kind(ButtonKind::Quiet)
                .size(ButtonSize::Md)
                .icon("x")
                .icon_position(enzui_widgets::button::ButtonIconPosition::Only)
                .tooltip(enzui_core::operation_activity::ActivityControl::Cancel.label())
                .aria_label(op.id().cancel_aria_label(op.kind.verb(), &op.name))
                .enabled(op.cancel.cancel_is_actionable())
                .into_element(),
            );
        }
    } else if installed {
        let can_update = has_update && update_authorization.is_allowed();
        if can_update {
            buttons.push(
                enz_button(
                    crate::i18n::label("Update"),
                    Message::UpdateRequested(app_id.to_owned(), name.to_owned()),
                )
                .kind(ButtonKind::Accent)
                .size(ButtonSize::Md)
                .min_width(PRIMARY_ACTION_MIN_WIDTH)
                .id(iced::advanced::widget::Id::from(format!(
                    "store-detail-update:{app_id}"
                )))
                .aria_label(crate::i18n::format(
                    crate::i18n::localizer(),
                    "Update {name}",
                    &[("name", &(name))],
                ))
                .into_element(),
            );
        } else if has_update && let Some(refusal) = update_authorization.refusal() {
            // A pending update this build cannot apply stays disclosed, in the
            // update's own vocabulary. The Remove slot answers a different
            // question and may say something different.
            notes.push(super::super::library::refusal_chip(refusal));
        }
        // Open is the primary action unless an update is waiting, when it
        // steps down to a quiet button beside Update.
        buttons.push(
            enz_button(
                crate::i18n::label("Open"),
                Message::LaunchInstalled(app_id.to_owned()),
            )
            .kind(if can_update {
                ButtonKind::Quiet
            } else {
                ButtonKind::Accent
            })
            .size(ButtonSize::Md)
            .min_width(PRIMARY_ACTION_MIN_WIDTH)
            .enabled(
                store.launching_app.is_none()
                    && store.installed_scope(app_id) != crate::services::InstallScope::Unknown,
            )
            .id(format!("store-detail-open:{app_id}").into())
            .into_element(),
        );
        match mutation.refusal() {
            None => {
                // Read the raw installed list so Library and Detail disclose the same surviving copy.
                if store.system_copy_survives_removal(app_id) {
                    notes.push(super::super::library::notice_chip(
                        crate::services::SYSTEM_COPY_REMAINS,
                    ));
                }
                buttons.push(
                    enz_button(
                        crate::i18n::label("Remove"),
                        Message::UninstallConfirmRequested(app_id.to_owned(), name.to_owned()),
                    )
                    .kind(ButtonKind::QuietDestructive)
                    .size(ButtonSize::Md)
                    .icon("trash-2")
                    .icon_position(enzui_widgets::button::ButtonIconPosition::Only)
                    .tooltip(crate::i18n::label("Remove"))
                    .aria_label(crate::i18n::format(
                        crate::i18n::localizer(),
                        "Remove {name}",
                        &[("name", &(name))],
                    ))
                    .into_element(),
                );
            }
            // The same chip the Library row shows, from the same helper and
            // the same `Refusal` value, so the two surfaces cannot state the
            // refusal differently.
            Some(refusal) => notes.push(super::super::library::refusal_chip(refusal)),
        }
    } else {
        match install_authorization.refusal() {
            None => buttons.push(
                enz_button(
                    crate::i18n::label("Install"),
                    Message::InstallRequested(app_id.to_owned(), name.to_owned()),
                )
                .kind(ButtonKind::Accent)
                .size(ButtonSize::Md)
                .min_width(PRIMARY_ACTION_MIN_WIDTH)
                .id(iced::advanced::widget::Id::from(format!(
                    "store-detail-install:{app_id}"
                )))
                .aria_label(crate::i18n::format(
                    crate::i18n::localizer(),
                    "Install {name}",
                    &[("name", &(name))],
                ))
                .into_element(),
            ),
            Some(refusal) => notes.push(super::super::library::refusal_chip(refusal)),
        }
    }

    let align = if align_end {
        Alignment::End
    } else {
        Alignment::Start
    };
    let mut group = column![].spacing(SpaceToken::Sm.px()).align_x(align);
    if !buttons.is_empty() {
        group = group.push(
            iced::widget::Row::with_children(buttons)
                .spacing(SpaceToken::Sm.px())
                .align_y(Alignment::Center),
        );
    }
    for note in notes {
        group = group.push(note);
    }
    group.into()
}

/// The content rating: the age band as a badge beside the per-category
/// descriptions the OARS map declares.
fn content_rating_block<'a>(store: &'a Store, has_entry: bool) -> Element<'a, Message> {
    let mut block = column![chrome::section_label(crate::i18n::label("Content Rating"))]
        .spacing(SpaceToken::Sm.px())
        .width(Length::Fill);
    if !has_entry {
        return block.into();
    }
    // Ratings use the fetched OARS map rather than inferring a universal age from catalogue tags.
    let ctx = store.detail_context();
    let age = ctx.map(|c| c.age_rating.as_str()).unwrap_or_default();
    let settled = matches!(
        ctx.map(|context| &context.metadata_state),
        Some(
            crate::services::DetailRouteState::Known
                | crate::services::DetailRouteState::KnownEmpty
                | crate::services::DetailRouteState::CachedStale { .. }
        )
    );
    // The headline is the age band the map actually yields. Empty means the
    // map carried no recognised severity - "Not rated", not a reassuring
    // default.
    let overall_text = match ctx.map(|context| &context.metadata_state) {
        Some(crate::services::DetailRouteState::Failed) => {
            crate::i18n::label("Content rating unavailable.").to_owned()
        }
        Some(crate::services::DetailRouteState::Loading) | None => {
            crate::i18n::label("Loading content rating…").to_owned()
        }
        _ if age.is_empty() => {
            crate::model::content_rating_overall(crate::i18n::label("Not rated"))
        }
        _ => crate::model::content_rating_overall(age),
    };
    let mut lines = column![
        enz_text(overall_text)
            .size(TextSize::Body)
            .tone(TextTone::Default)
    ]
    .spacing(SpaceToken::Xxs.px())
    .width(Length::Fill);
    // Per-category descriptions: "Mild cartoon violence". The description
    // already names the category and its severity.
    if let Some(rating) = ctx.map(|c| &c.content_rating) {
        for oars_row in crate::services::categorize_oars(rating) {
            lines = lines.push(
                enz_text(oars_row.description)
                    .size(TextSize::Footnote)
                    .tone(TextTone::Muted),
            );
        }
    }
    if matches!(
        ctx.map(|context| &context.metadata_state),
        Some(crate::services::DetailRouteState::CachedStale { .. })
    ) {
        lines = lines.push(chrome::muted_line(crate::i18n::label(
            "Showing cached metadata while refreshing.",
        )));
    }
    let body: Element<'a, Message> = if settled && !age.is_empty() {
        row![age_badge(age), lines]
            .spacing(SpaceToken::Md.px())
            .align_y(Alignment::Center)
            .into()
    } else {
        lines.into()
    };
    block = block.push(body);
    block.into()
}

/// The age band ("12+") in an outlined square, the way store pages show it.
fn age_badge<'a>(age: &str) -> Element<'a, Message> {
    let label = enz_text(age.to_owned())
        .size(TextSize::Headline)
        .weight(iced::font::Weight::Semibold)
        .tone(TextTone::Default)
        .announce(false);
    enz_surface(
        container(label)
            .center_x(Length::Fixed(AGE_BADGE_PX))
            .center_y(Length::Fixed(AGE_BADGE_PX)),
    )
    .tone(SurfaceTone::Transparent)
    .border_style(BorderStyle::Strong)
    .radius_size(enzui_widgets::surface::RadiusSize::Lg)
    .into_element()
}

const AGE_BADGE_PX: f32 = 44.0;

/// The user ratings: the average as a large figure beside its stars and
/// review count, with the histogram one click away.
fn user_ratings_block<'a>(store: &'a Store) -> Element<'a, Message> {
    // No reviews uses the unrated announcement without a fabricated NumericValue; the histogram stays opt-in.
    let mut block = column![chrome::section_label(crate::i18n::label("User Ratings"))]
        .spacing(SpaceToken::Sm.px());
    let ratings_context = store.detail_context();
    if let Some(counts) = ratings_context.and_then(|c| c.ratings.as_ref()) {
        let rated = counts.total > 0;
        let mut rating = enz_rating(if rated { counts.average() } else { f32::NAN })
            .label(crate::i18n::label("Review score"));
        if rated {
            rating = rating.value_text(format!(
                "{} \u{b7} {}",
                counts.format_average(),
                counts.format_total()
            ));
            let summary = row![
                enz_text(counts.format_average())
                    .size(TextSize::Title1)
                    .weight(iced::font::Weight::Bold)
                    .tone(TextTone::Default)
                    .announce(false),
                column![
                    rating.into_element(),
                    enz_text(counts.format_total())
                        .size(TextSize::Footnote)
                        .tone(TextTone::Muted)
                        .announce(false),
                ]
                .spacing(SpaceToken::Xxs.px()),
            ]
            .spacing(SpaceToken::Md.px())
            .align_y(Alignment::Center);
            block = block.push(summary);

            let open = store.ratings_breakdown_open();
            // The reveal toggle: stars + count are the glance, the histogram
            // is opt-in.
            let toggle = enz_button(
                ratings_breakdown_toggle_label(open),
                Message::ToggleRatingsBreakdown,
            )
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            // `chart-no-axes-column` is the canonical name for the shape
            // upstream Lucide used to call `BarChart3`; the old name is
            // absent from the registry. Covered by
            // `every_icon_this_app_renders_resolves_in_the_registry`,
            // which opens the breakdown to reach this line.
            .icon("chart-no-axes-column");
            // Vertical-size motion changes reserved layout height so the histogram cannot overlap attribution.
            let mut histogram = column![].spacing(SpaceToken::Sm.px()).width(Length::Fill);
            // Histogram: 5 down to 1, the conventional reading order.
            for star in (1..=5u8).rev() {
                histogram = histogram.push(
                    enz_progress_bar(counts.bar_fraction(star))
                        .label(&format!("{star} star"))
                        .value_text(&counts.bar(star).to_string())
                        .into_element(),
                );
            }
            // The gap above the bars lives inside the motion, so the closed
            // histogram leaves no empty band between the toggle and the
            // attribution under it.
            let histogram =
                container(histogram).padding(iced::Padding::new(0.0).top(SpaceToken::Sm.px()));
            let histogram_el: Element<'a, Message> =
                motion(histogram, open, MotionEffect::Size(MotionAxis::Vertical)).into();
            block = block.push(column![toggle, histogram_el].width(Length::Fill));
        } else {
            block = block.push(rating.into_element());
        }
    }
    match ratings_context.map(|context| &context.ratings_state) {
        Some(crate::services::DetailRouteState::Failed) => {
            block = block.push(chrome::muted_line(crate::i18n::label(
                "Ratings unavailable.",
            )));
        }
        Some(crate::services::DetailRouteState::KnownEmpty) => {
            block = block.push(chrome::muted_line(crate::i18n::label("No ratings yet.")));
        }
        Some(crate::services::DetailRouteState::CachedStale { age_secs }) => {
            block = block.push(chrome::muted_line(crate::i18n::format(
                crate::i18n::localizer(),
                "Cached ratings from {} ago; refresh failed.",
                &[(
                    "arg0",
                    &(crate::model::format_age(age_secs.saturating_mul(1000))),
                )],
            )));
        }
        Some(crate::services::DetailRouteState::Loading) | None => {
            // The average with its stars, and the breakdown toggle under it.
            let label = crate::i18n::label("Loading ratings…");
            block = block.push(
                column![
                    chrome::skeleton_block(Length::Fixed(160.0), 34.0, label),
                    chrome::skeleton_block(Length::Fixed(176.0), 26.0, label),
                ]
                .spacing(SpaceToken::Sm.px()),
            );
        }
        Some(crate::services::DetailRouteState::Known) => {}
    }
    block = block.push(
        enz_text(ratings_attribution().to_owned())
            .size(TextSize::Footnote)
            .color_resolved(|t| crate::theme::palette(t).text_muted),
    );
    block.width(Length::Fill).into()
}
