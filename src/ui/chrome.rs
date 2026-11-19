//! The small, flat vocabulary every view is built from.
//!
//! See [`crate::ui`] for why this is deliberately not cards and badges.

use enzui_core::operation_activity::ActivityControl;
use enzui_theme::spacing::SpaceToken;
use enzui_widgets::banner::{BannerTone, EnzBanner, enz_banner};
use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_widgets::icon_label::{IconLabelDisplay, IconLabelTone, enz_icon_label};
use enzui_widgets::list_row_surface::enz_list_row_surface;
use enzui_widgets::max_width::max_width;
use enzui_widgets::scroll_column::DEFAULT_SCROLLBAR_GUTTER;
use enzui_widgets::section_header::enz_section_header;
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Border, Color, Element, Length, Padding};

/// Compact installation action used throughout Discover.
pub(crate) fn discover_install<'a>(
    entry: &crate::services::AppEntry,
) -> Element<'a, crate::Message> {
    enzui_widgets::toolbar_icon::enz_toolbar_icon(
        "download",
        crate::Message::InstallRequested(entry.id.clone(), entry.name.clone()),
    )
    .variant(enzui_widgets::toolbar_icon::ToolbarIconVariant::Standard)
    .aria_label(crate::i18n::format(
        crate::i18n::localizer(),
        "Install {}",
        &[("arg0", &entry.name)],
    ))
    .tooltip(crate::i18n::label("Install"))
    .into()
}

// ---------------------------------------------------------------------------
// Failure alert
// ---------------------------------------------------------------------------

pub(crate) const FAILURE_BANNER_TONE: BannerTone = BannerTone::Danger;

/// Build the app's non-dismissible Danger alert and cap its complete announced
/// label, including the banner's severity prefix and title/body punctuation.
pub(crate) fn failure_banner<'a, Message: 'a + Clone>(
    title: impl Into<String>,
    body: Option<String>,
) -> EnzBanner<'a, Message> {
    let (title, body) = failure_copy(title, body);
    let banner = enz_banner(title)
        .tone(FAILURE_BANNER_TONE)
        .dismissible(false);
    match body {
        Some(body) => banner.body(body),
        None => banner,
    }
}

/// A failure's localized, sanitized title and body, capped so the complete
/// announced label (severity prefix and punctuation included) stays bounded.
/// Shared by [`failure_banner`] and the failure toasts.
pub(crate) fn failure_copy(
    title: impl Into<String>,
    body: Option<String>,
) -> (String, Option<String>) {
    use crate::services::flatpak_failure::{
        RENDERED_FAILURE_CHAR_CAP, cap_chars, sanitize_diagnostic,
    };

    const SEVERITY_PREFIX: &str = "Error: ";
    const TITLE_BODY_SEPARATOR: &str = ". ";

    let title = sanitize_diagnostic(&crate::i18n::text(&title.into()));
    let title_budget = RENDERED_FAILURE_CHAR_CAP.saturating_sub(SEVERITY_PREFIX.chars().count());
    let title = cap_chars(&title, title_budget);
    let body = body.map(|body| sanitize_diagnostic(&crate::i18n::text(&body)));
    let body_budget = RENDERED_FAILURE_CHAR_CAP.saturating_sub(
        SEVERITY_PREFIX.chars().count()
            + title.chars().count()
            + TITLE_BODY_SEPARATOR.chars().count(),
    );
    let body = body
        .filter(|body| !body.is_empty() && body_budget > 0)
        .map(|body| cap_chars(&body, body_budget));
    (title, body)
}

// ---------------------------------------------------------------------------
// Icon chip — the non-badge metadata affordance
// ---------------------------------------------------------------------------

/// Colour role for an [`icon_chip`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum ChipTone {
    #[default]
    Muted,
    Success,
    Warning,
    /// An operation that ended in an error. Distinct from `Warning`, which
    /// marks a partial or recoverable state.
    Danger,
    Accent,
}

impl ChipTone {
    // Pin label and icon ink together; semantic tones use separate contrast-adjusted text colours.
    fn icon_label_tone(self) -> IconLabelTone {
        match self {
            ChipTone::Muted => IconLabelTone::Muted,
            ChipTone::Success => IconLabelTone::Success,
            ChipTone::Warning => IconLabelTone::Warning,
            ChipTone::Danger => IconLabelTone::Danger,
            ChipTone::Accent => IconLabelTone::Accent,
        }
    }
}

pub(crate) const CHIP_ICON_PX: f32 = 13.0;

/// Render an unfilled, co-centered icon and label with a semantic tone.
/// The foreground resolves against the current theme so a theme change preserves contrast.
pub(crate) fn icon_chip<'a, Message: 'a>(
    icon: &str,
    label: impl Into<String>,
    tone: ChipTone,
) -> Element<'a, Message> {
    state_chip(icon, label, tone, true)
}

/// A chip whose `label` is always its accessible name, drawn only when
/// `labelled`.
///
/// With `labelled` false the chip renders its icon alone and keeps the name.
/// An empty `label` publishes no accessibility node at all.
pub(crate) fn state_chip<'a, Message: 'a>(
    icon: &str,
    label: impl Into<String>,
    tone: ChipTone,
    labelled: bool,
) -> Element<'a, Message> {
    enz_icon_label(icon, label.into())
        .tone(tone.icon_label_tone())
        .display(if labelled {
            IconLabelDisplay::Both
        } else {
            IconLabelDisplay::IconOnly
        })
        // Shrink to content, matching the old `row![glyph, text]` (which was
        // `Length::Shrink`). `enz_icon_label` defaults to `fill = true`, which
        // gives the label `.full_width()` and lets `single_line_elided()`
        // truncate a chip label to a `…` inside a wide row — turning e.g.
        // "App ID: org.gimp.GIMP" into elided text the detail-page render test
        // can no longer find. A metadata chip is content-sized, so fill off.
        .fill(false)
        .icon_size(CHIP_ICON_PX)
        .co_centre_to_icon()
        // Footnote (12) label, matching the old `.size(TextSize::Footnote)`.
        .size(TextSize::Footnote)
        .spacing(SpaceToken::Xs.px())
        .into()
}

/// Right inset reserved by the framework scrollbar overlay.
/// Scrolling rows and adjacent non-scrolling dividers share this reservation.
pub(crate) const SCROLLBAR_GUTTER: f32 = DEFAULT_SCROLLBAR_GUTTER;

pub(crate) const ROW_HEIGHT: f32 = 64.0;

/// Render a validated image handle or a lettered placeholder; production callers cannot provide undecoded cache paths.
pub(crate) fn app_icon<'a, Message: 'a>(
    image: Option<&crate::bounded_image::RenderImage>,
    name: &str,
    px: f32,
) -> Element<'a, Message> {
    if let Some(image) = image {
        // Keep remote icons transparent and unrounded while the framework publishes their accessible app name.
        return enzui_widgets::image_widget::enz_image_handle(image.icon_handle())
            .fit(enzui_widgets::image_widget::EnzImageFit::Contain)
            .alt_text(name.to_owned())
            .placeholder_background(Color::TRANSPARENT)
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .into();
    }
    let letter = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().to_string());
    container(
        enz_text(letter)
            .size_px(px * 0.45)
            .weight(iced::font::Weight::Semibold)
            .color_resolved(|theme| crate::theme::palette(theme).text_muted),
    )
    .width(Length::Fixed(px))
    .height(Length::Fixed(px))
    .align_x(iced::alignment::Horizontal::Center)
    .align_y(iced::alignment::Vertical::Center)
    .style(|theme: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(
            crate::theme::palette(theme).surface_alt,
        )),
        border: Border {
            radius: 9.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

pub(crate) const ROW_CONTENT_MAX_WIDTH: f32 = 960.0;

/// Compose an app-row subtitle from summary, metadata chips, and an app-id fallback.
/// Empty content uses the fallback; non-stacked rows use one summary line.
/// Stacking is reserved for wide rows because narrow rows have a fixed single-line height.
#[must_use]
pub(crate) struct AppRowSubtitle<'a, Message: 'a + Clone> {
    chips: Vec<Element<'a, Message>>,
    summary: String,
    fallback: String,
    stack: bool,
}

impl<'a, Message: 'a + Clone> AppRowSubtitle<'a, Message> {
    /// A new subtitle whose empty-band fallback is `fallback` — usually the
    /// app id, which is unique and non-empty for every real `AppEntry`.
    pub(crate) fn new(fallback: impl Into<String>) -> Self {
        Self {
            chips: Vec::new(),
            summary: String::new(),
            fallback: fallback.into(),
            stack: true,
        }
    }

    /// The app's one-line summary. Empty string = no summary line.
    pub(crate) fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Whether a non-empty summary is allowed to *stack* above the chip strip
    /// (two lines). At narrow / compact rungs a caller passes `false` so the
    /// row fits the fixed 64 px [`ROW_HEIGHT`] with the summary on one line
    /// and the chips dropped in favour of the summary text — the same fit
    /// search enforced with `!density.shows_installs()`.
    pub(crate) fn stack(mut self, stack: bool) -> Self {
        self.stack = stack;
        self
    }

    /// Append a chip without requiring callers to track strip emptiness separately.
    pub(crate) fn push_chip(mut self, chip: Element<'a, Message>) -> Self {
        self.chips.push(chip);
        self
    }

    /// Append `chip` only when `cond` is true; the strip's own emptiness is
    /// still the builder's answer.
    pub(crate) fn push_chip_if<F>(self, cond: bool, chip: F) -> Self
    where
        F: FnOnce() -> Element<'a, Message>,
    {
        if cond { self.push_chip(chip()) } else { self }
    }

    /// Compose the subtitle Element. The empty-band case is impossible by
    /// construction — the `summary_empty && chips_empty` branch is taken
    /// first and falls back to `muted_line(fallback)`.
    pub(crate) fn build(self) -> Element<'a, Message> {
        let summary_empty = self.summary.is_empty();
        let chips_empty = self.chips.is_empty();

        if summary_empty && chips_empty {
            return muted_line(self.fallback);
        }

        let mut strip = row![]
            .spacing(SpaceToken::Md.px())
            .align_y(Alignment::Center);
        for chip in self.chips {
            strip = strip.push(chip);
        }

        if summary_empty {
            return strip.into();
        }
        if !self.stack {
            // Narrow / compact: one line only. Stacking summary above chips
            // would overflow the fixed 64 px `ROW_HEIGHT`.
            return muted_line(self.summary);
        }

        column![muted_line(self.summary), strip]
            .spacing(SpaceToken::Xs.px())
            .width(Length::Fill)
            .into()
    }
}

/// A keyboard-activatable fixed-height app row with centered readable-width content; interactive children retain their own events.
pub(crate) fn app_row<'a, Message: 'a + Clone>(
    icon: Option<&crate::bounded_image::RenderImage>,
    name: &str,
    subtitle: Element<'a, Message>,
    trailing: Element<'a, Message>,
    on_activate: Message,
    id_prefix: Option<String>,
) -> Element<'a, Message> {
    let title = enz_text(name.to_owned())
        .size(TextSize::Body)
        .weight(iced::font::Weight::Medium)
        .single_line_elided()
        .full_width()
        .color_resolved(|theme| crate::theme::palette(theme).text);

    // A custom 3px optical gap groups name and subtitle between the available 2px and 4px token rungs.
    let text_block = column![title, subtitle]
        .spacing(SpaceToken::Custom(3.0).px())
        .width(Length::Fill);

    // Build the entire interactive row at the readable measure. The old
    // composition capped only this content and then wrapped it in a full-width
    // `enz_list_row_surface`; on a wide Library page, hovering empty gutter
    // hundreds of pixels from the app still lit the whole viewport-width band.
    let body: Element<'a, Message> = container(
        row![app_icon(icon, name, ROW_ICON_PX), text_block, trailing]
            .spacing(SpaceToken::Space7.px())
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .padding(Padding::from([0.0, SpaceToken::Xs.px()]))
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill)
    .into();

    let surface = enz_list_row_surface(body, on_activate)
        .content_padding(0.0)
        .label(name.to_owned())
        // The responsive grid owns card edges, so row surfaces must decline an additional outer gutter.
        .outer_gutter(0.0)
        .row_height(ROW_HEIGHT)
        .width(Length::Fill);
    let surface = match id_prefix {
        Some(prefix) => surface.id_prefix(prefix),
        None => surface,
    }
    .into_element();

    readable_measure(surface)
}

/// Centre `content` at the shared readable measure ([`ROW_CONTENT_MAX_WIDTH`]).
///
/// App rows already sit on this measure; a page header, filter strip or
/// detail body that spans the full pane beside them starts at a different
/// left edge than the rows it introduces once the window is wider than the
/// measure. Every list page routes its head and its rows through this one
/// helper so their edges cannot drift apart.
pub(crate) fn readable_measure<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(
        row![
            Space::new().width(Length::Fill),
            max_width(content.into(), ROW_CONTENT_MAX_WIDTH),
            Space::new().width(Length::Fill),
        ]
        .align_y(Alignment::Center)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .into()
}

/// Muted one-line body text that **fills** its slot and elides — the subtitle
/// of an [`app_row`], where the text column is the flexible part of the row.
/// A pulsing placeholder of a fixed height that holds the place of content
/// still loading, so the page does not move when it arrives. `label` names
/// what is loading for assistive technology.
pub(crate) fn skeleton_block<'a, Message: 'a>(
    width: Length,
    height: f32,
    label: impl Into<String>,
) -> Element<'a, Message> {
    use enzui_widgets::skeleton::{SkeletonKind, enz_skeleton};
    container(
        enz_skeleton(SkeletonKind::Card)
            .fill_height(true)
            .label(label)
            .into_element(),
    )
    .width(width)
    .height(Length::Fixed(height))
    .into()
}

pub(crate) fn muted_line<'a, Message: 'a>(s: impl Into<String>) -> Element<'a, Message> {
    enz_text(s.into())
        .size(TextSize::Footnote)
        .single_line_elided()
        .full_width()
        .color_resolved(|theme| crate::theme::palette(theme).text_muted)
        .into()
}

/// Muted one-line body text that **hugs** its content.
///
/// The distinction is load-bearing, not stylistic. `Length::Fill` on a label
/// sitting beside a fixed control makes the two *share* the row, so at 375 px
/// "91 apps · 17.9 GB" elided to "91 a…" with ~180 px of empty space to its
/// right — measured in `mobile-dark.png`. A caption that is not the flexible
/// part of its row must shrink to its content and let an explicit spacer take
/// the slack.
pub(crate) fn muted_caption<'a, Message: 'a>(s: impl Into<String>) -> Element<'a, Message> {
    enz_text(s.into())
        .size(TextSize::Footnote)
        .single_line_elided()
        .width(Length::Shrink)
        .color_resolved(|theme| crate::theme::palette(theme).text_muted)
        .into()
}

/// The height of the progress control that replaces an Install button.
///
/// Equal to the medium button rung, so replacing the action in place cannot
/// move the header or row around it.
pub(crate) fn progress_action_height() -> f32 {
    enzui_widgets::button::ButtonSize::Md.tokens().height
}

/// Builds the compact status that replaces an action while work is running:
/// a [`progress_pill`] at the medium button height.
pub(crate) fn progress_action<'a, Message: 'a>(
    verb: &str,
    subject: &str,
    completion: enzui_widgets::gauge::Sampled<f32>,
    width: Length,
) -> Element<'a, Message> {
    progress_pill(verb, subject, completion, width, progress_action_height())
}

/// The button an operation replaces, turned into its progress: a pill of the
/// button's height on a faint track, filling from the leading end with a
/// tint of the action's colour (red for a removal), "Updating 38%" across it.
/// Before Flatpak reports a figure the track stays empty and reads
/// "Updating…".
pub(crate) fn progress_pill<'a, Message: 'a>(
    verb: &str,
    subject: &str,
    completion: enzui_widgets::gauge::Sampled<f32>,
    width: Length,
    height: f32,
) -> Element<'a, Message> {
    let fraction = match completion {
        enzui_widgets::gauge::Sampled::Sampled(value) => Some(value.clamp(0.0, 1.0)),
        enzui_widgets::gauge::Sampled::Unsampled => None,
    };
    let caption = match fraction {
        Some(value) => format!("{verb} {}%", (value * 100.0).round() as u32),
        None => format!("{verb}…"),
    };
    let removal = verb == crate::model::OpKind::Uninstall.verb();
    let radius = height / 2.0;

    let track = container(Space::new())
        .width(width)
        .height(Length::Fixed(height))
        .style(move |theme: &iced::Theme| {
            let p = crate::theme::palette(theme);
            iced::widget::container::Style {
                background: Some(Color { a: 0.08, ..p.text }.into()),
                border: Border::default().rounded(radius),
                ..Default::default()
            }
        });

    // The fill and the room left, as shares of the pill's width, so the
    // pill can take any width its slot gives it.
    let done = fraction.map_or(0, |value| (value * 1000.0).round() as u16);
    let fill: Element<'a, Message> = if done == 0 {
        Space::new().into()
    } else {
        row![
            container(Space::new())
                .width(Length::FillPortion(done))
                .height(Length::Fill)
                .style(move |theme: &iced::Theme| {
                    let p = crate::theme::palette(theme);
                    let tint = if removal { p.danger } else { p.accent };
                    iced::widget::container::Style {
                        background: Some(Color { a: 0.34, ..tint }.into()),
                        border: Border::default().rounded(radius),
                        ..Default::default()
                    }
                }),
            Space::new().width(Length::FillPortion(1000 - done.min(1000))),
        ]
        .into()
    };

    let label = container(
        enz_text(caption)
            .size(TextSize::Caption)
            .weight(iced::font::Weight::Semibold)
            .single_line_elided()
            .color_resolved(|theme| crate::theme::palette(theme).text),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .padding(
        Padding::new(0.0)
            .left(SpaceToken::Sm.px())
            .right(SpaceToken::Sm.px()),
    );

    let control: Element<'a, Message> = iced::widget::stack![track, fill, label].into();

    enzui_widgets::semantics_container::SemanticsContainer::new(
        control,
        enzui_core::semantics::Semantics::new(enzui_core::semantics::Role::Group)
            .label(format!("{verb} {subject}")),
    )
    .into()
}

/// Icon-only Cancel for an operation row whose subtitle carries the progress.
///
/// Same message, same accessible name and the same disabled-while-reaping
/// state; the visible "Cancelling…" is the row's subtitle, which already
/// carries the operation's message.
pub(crate) fn cancel_icon<'a>(
    op: &crate::model::OperationState,
) -> Option<Element<'a, crate::Message>> {
    if op.finished || !op.cancel.offers_cancel() {
        return None;
    }
    Some(
        enzui_widgets::toolbar_icon::enz_toolbar_icon(
            "x",
            crate::Message::CancelOperationRequested(op.id()),
        )
        .aria_label(op.id().cancel_aria_label(op.kind.verb(), &op.name))
        .tooltip(ActivityControl::Cancel.label())
        .enabled(op.cancel.cancel_is_actionable())
        .into_element(),
    )
}

/// What an action slot shows while the reducer owns an operation for that app.
///
/// Running renders [`progress_action`], failure renders an alert carrying the
/// reason, and completion renders a past-tense status. Removal completion uses
/// a neutral trash mark while install and update completion use success checks.
/// `labelled` may hide completion text in compact app rows; failures stay named.
pub(crate) fn operation_slot<'a, Message: 'a>(
    op: &crate::model::OperationState,
    width: Length,
    labelled: bool,
) -> Element<'a, Message> {
    if let Some(error) = &op.error {
        use enzui_core::semantics::{Role, Semantics};

        // An icon-only failure is unreadable, so this branch ignores
        // `labelled`. The tooltip is the pointer reading of the reason; the
        // node below is every other reading, since `enz_tooltip` publishes no
        // semantics of its own.
        let chip = enzui_widgets::tooltip::enz_tooltip(
            icon_chip(
                "circle-alert",
                crate::i18n::label("Failed"),
                ChipTone::Danger,
            ),
            error.clone(),
            iced::widget::tooltip::Position::Top,
        );
        return enzui_widgets::semantics_container::SemanticsContainer::leaf(
            chip,
            Semantics::new(Role::Alert)
                .label(format!("{} {} failed", op.kind.verb(), op.name))
                .description(error.clone()),
        )
        .into();
    }
    if op.finished {
        if op.cancel == crate::model::CancelState::Cancelled {
            let label = if labelled {
                crate::i18n::label("Cancelled")
            } else {
                ""
            };
            return icon_chip("x", label, ChipTone::Muted);
        }
        let label = if labelled { op.kind.past() } else { "" };
        return match op.kind {
            crate::model::OpKind::Uninstall => icon_chip("trash-2", label, ChipTone::Muted),
            crate::model::OpKind::Install | crate::model::OpKind::Update => {
                icon_chip("check", label, ChipTone::Success)
            }
        };
    }
    progress_action(op.kind.verb(), &op.name, op.completion(), width)
}

/// Render an uppercased section label with theme-reactive subtle ink.
/// Shrink width leaves room for trailing actions in the same row.
pub(crate) fn section_label<'a, Message: 'a>(s: impl Into<String>) -> Element<'a, Message> {
    enz_section_header(s.into().to_uppercase())
        .tone(TextTone::Subtle)
        .width(Length::Shrink)
        .into()
}

#[path = "chrome/metrics.rs"]
mod metrics;

pub(crate) use metrics::{
    ROW_GAP, ROW_ICON_PX, catalog_overscan_rows, hairline, page_inset, row_pitch,
};

/// A compact select: the current value as text with a chevron, opening a
/// menu of the options with the current one checked. The text-and-chevron
/// grammar of a preferences row's value menu (Music's equalizer presets),
/// for a header with no row around it.
pub(crate) fn compact_select<'a, Message: Clone + 'a>(
    label: &str,
    current: &str,
    open: bool,
    on_open: Message,
    on_dismiss: Message,
    options: Vec<(String, Message)>,
    id: &'static str,
) -> Element<'a, Message> {
    use enzui_widgets::focus_button::FocusStatus;
    let body = container(
        row![
            enz_text(current.to_owned())
                .size(TextSize::Footnote)
                .tone(TextTone::Muted)
                .single_line_elided(),
            enzui_widgets::icon::enz_icon("navigation.down")
                .size(12.0)
                .tint_resolved(|t| crate::theme::palette(t).text_muted),
        ]
        .spacing(SpaceToken::Xs.px())
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([3.0, 8.0]));
    let trigger_id = iced::advanced::widget::Id::new(id);
    let trigger = enzui_widgets::focus_button::focus_button_maybe(body, Some(on_open))
        .clip(true)
        .style_focus(move |theme: &iced::Theme, status| {
            let p = crate::theme::palette(theme);
            let lit = matches!(status, FocusStatus::Hovered | FocusStatus::Pressed) || open;
            let focused = matches!(status, FocusStatus::Focused);
            iced::widget::button::Style {
                background: Some(iced::Background::Color(if lit {
                    p.surface_hover
                } else {
                    Color::TRANSPARENT
                })),
                text_color: p.text,
                border: Border {
                    color: if focused {
                        p.focus_ring
                    } else {
                        Color::TRANSPARENT
                    },
                    width: if focused { 1.5 } else { 0.0 },
                    radius: 6.0.into(),
                },
                ..iced::widget::button::Style::default()
            }
        })
        .semantics(
            enzui_core::semantics::Semantics::new(enzui_core::semantics::Role::ComboBox)
                .label(label.to_owned())
                .value(current.to_owned())
                .default_action()
                .expanded(open),
        )
        .expand_alias(trigger_id.clone())
        .id(trigger_id.clone())
        .relinquish_focus_while(open)
        .reclaim_focus_on_return((!open).then_some(trigger_id.clone()));
    let popup: Option<Element<'a, Message>> = open.then(|| {
        let mut menu = enzui_widgets::menu::enz_menu()
            .hug_content(true)
            .hug_max_width(0.0)
            .label(label.to_owned())
            .id(enzui_core::identity::derive_id(id, "menu"))
            .id_prefix(format!("{id}-menu-item"))
            .roving_self(true)
            .active(current.to_owned())
            .focus_return_to(trigger_id.clone());
        for (option, message) in options {
            let checked = option == current;
            menu = menu.push_item(
                enzui_widgets::menu::EnzMenuItem::command(option.clone(), option, message)
                    .check(if checked {
                        enzui_core::menu_model::CheckState::Checked
                    } else {
                        enzui_core::menu_model::CheckState::Unchecked
                    })
                    .radio(true),
            );
        }
        menu.into()
    });
    let placement = enzui_core::placement::Placement::new(enzui_core::placement::Side::Bottom)
        .alignment(enzui_core::placement::Alignment::End)
        .gap(4.0);
    enzui_widgets::anchored_overlay::dismissible_anchored_menu(
        trigger,
        popup,
        placement,
        move || on_dismiss.clone(),
    )
}
