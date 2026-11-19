//! Responsive Store navigation and actions.
//!
//! Navigation moves to a subtitle row on compact windows; the framework
//! measures and folds individual titlebar commands.

use crate::model::LibrarySort;
use crate::{Message, Route};

use enzui_core::{Action, Icon, action_overflow::Descriptor};
use enzui_theme::SizingTokens;
use enzui_theme::spacing::SpaceToken;
use enzui_widgets::action_bar::ActionItem;
use enzui_widgets::tab_pill::TabSelectionBounds;
use enzui_widgets::title_bar::enz_title_bar;
use enzui_widgets::toolbar_icon::enz_toolbar_icon;
use enzui_widgets::view_switcher::{
    SwitcherPage, SwitcherPresentation, SwitcherTokens, enz_view_switcher,
};
use iced::widget::{Space, container};
use iced::{Element, Length, Padding};

// Keep a 44px titlebar drag target independent of compact control sizes.
pub(crate) const NAV_ROW_HEIGHT: f32 = 44.0;

/// Return horizontal bar padding in logical pixels.
pub(crate) fn bar_padding_x() -> f32 {
    SpaceToken::Md.px()
}

pub(crate) const SEARCH_PLACEHOLDER: &str = "Search apps and descriptions…";

const STACKED_TAB_PADDING_X: SpaceToken = SpaceToken::Space5;

/// Return horizontal padding for stacked tabs in logical pixels.
pub(crate) fn stacked_tab_padding_x() -> f32 {
    STACKED_TAB_PADDING_X.px()
}

/// Return the window-width threshold for stacked navigation, in logical pixels.
pub(crate) fn stack_below_width() -> f32 {
    SizingTokens::default().layout_switch_stack
}

/// Navigation destinations in presentation order; search and details are separate routes.
pub(crate) const NAV_ROUTES: &[Route] = &[Route::Discover, Route::Categories, Route::Library];

/// The bar's layout modes. Search is always an icon with a popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarMode {
    /// Nav centred in the bar.
    Medium,
    /// Search as an icon, nav on its own full-width stretched row below.
    Stacked,
}

impl BarMode {
    pub(crate) fn for_width(width: f32) -> Self {
        if width >= stack_below_width() {
            BarMode::Medium
        } else {
            BarMode::Stacked
        }
    }
    pub(crate) fn nav_is_stacked(self) -> bool {
        matches!(self, BarMode::Stacked)
    }
    /// Reserve space for labels by hiding icons in the stacked layout.
    pub(crate) fn nav_shows_icons(self) -> bool {
        !matches!(self, BarMode::Stacked)
    }
}

fn nav_pages(with_icons: bool, working: bool) -> Vec<SwitcherPage> {
    NAV_ROUTES
        .iter()
        .map(|r| {
            let page = SwitcherPage::new(r.key(), r.title());
            let page = if *r == Route::Library && working {
                page.with_busy(true)
            } else {
                page
            };
            if with_icons {
                page.with_icon(r.icon())
            } else {
                page
            }
        })
        .collect()
}

/// Return the nav index, or `NAV_ROUTES.len()` to request an unselected tab.
pub(crate) fn nav_selected_index(route: Route) -> usize {
    NAV_ROUTES
        .iter()
        .position(|r| *r == route)
        .unwrap_or(NAV_ROUTES.len())
}

/// Stable widget id of the activity live region the indicator carries.
pub(crate) const ACTIVITY_REGION_ID: &str = "store-activity-region";

/// The live-region reading of the shared activity center.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NavActivityRegion {
    /// `"Activity: 1 running, 42%"` — the framework's summary sentence.
    pub label: String,
    /// Every revealed row's announcement, joined.
    pub description: String,
    /// The aggregate fraction, only when the center reports an honest one.
    pub fraction: Option<f32>,
}

/// What the activity indicator beside the Library tab shows.
///
/// The spinner is visible while an install, update or removal runs. Completion
/// remains in the live region while its activity row lingers. The polite region
/// exists only once the center has revealed work, so quick work neither
/// flashes nor chatters.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NavActivity {
    /// Operations still running.
    pub running: usize,
    /// The revealed activity center, read as one live region.
    pub region: Option<NavActivityRegion>,
}

impl NavActivity {
    /// Read the indicator off the live store.
    pub(crate) fn of(store: &crate::Store) -> Self {
        use enzui_widgets::activity_surface::{REGION_LABEL, summary_sentence};

        let running = store
            .operations
            .iter()
            .filter(|op| !op.finished && op.error.is_none())
            .count();
        let region = if store.activity_center.is_empty() {
            None
        } else {
            let view = store.activity_center.view();
            let description = view
                .rows
                .iter()
                .map(|row| row.announcement())
                .collect::<Vec<_>>()
                .join("; ");
            Some(NavActivityRegion {
                label: format!(
                    "{}: {}",
                    crate::i18n::label(REGION_LABEL),
                    summary_sentence(&view.summary)
                ),
                description,
                fraction: view
                    .summary
                    .fraction
                    .filter(|f| f.is_finite())
                    .map(|f| f.clamp(0.0, 1.0)),
            })
        };
        Self { running, region }
    }
}

fn activity_semantics(region: &NavActivityRegion) -> enzui_core::semantics::Semantics {
    use enzui_core::semantics::{Live, NumericValue, Role, Semantics};
    let mut semantics = Semantics::new(Role::Status)
        .label(region.label.clone())
        .description(region.description.clone())
        .live(Some(Live::Polite));
    if let Some(fraction) = region.fraction {
        semantics = semantics.numeric(NumericValue {
            value: f64::from(fraction),
            min: 0.0,
            max: 1.0,
            step: None,
        });
    }
    semantics
}

/// The framework owns the busy tab's measurement and spinner. Keep the app's
/// aggregate live region so completion and errors are still announced.
fn nav_with_activity<'a>(
    route: Route,
    mode: BarMode,
    activity: &NavActivity,
) -> Element<'a, Message> {
    let nav = nav_element(route, mode, activity.running > 0);
    match &activity.region {
        Some(region) => enzui_widgets::semantics_container::SemanticsContainer::new(
            nav,
            activity_semantics(region),
        )
        .id(iced::advanced::widget::Id::new(ACTIVITY_REGION_ID))
        .into(),
        None => nav,
    }
}

fn nav_element<'a>(route: Route, mode: BarMode, working: bool) -> Element<'a, Message> {
    let stretch = mode.nav_is_stacked();
    let tokens = if stretch {
        SwitcherTokens {
            tab_padding_x: stacked_tab_padding_x(),
            ..SwitcherTokens::ROUNDED_STRIP
        }
    } else {
        SwitcherTokens::ROUNDED_STRIP
    };
    let presentation = if stretch {
        SwitcherPresentation::Stretch
    } else {
        SwitcherPresentation::Automatic
    };

    enz_view_switcher(
        nav_pages(mode.nav_shows_icons(), working),
        nav_selected_index(route),
        |i| Message::Navigate(NAV_ROUTES.get(i).copied().unwrap_or(Route::Discover)),
    )
    .tokens(tokens)
    .presentation(presentation)
    .selection_bounds(TabSelectionBounds::Unselected)
    .tab_id_prefix("store-nav")
    .aria_label(crate::i18n::label("Primary navigation"))
    .into_element()
}

/// An app command keeps its stable inline target and the same meaning when folded.
fn icon_action<'a>(
    id: &'static str,
    icon: &'static str,
    label: String,
    message: Message,
    enabled: bool,
    active: bool,
) -> ActionItem<'a, Message> {
    ActionItem::with_inline(
        Descriptor::new(id),
        Action::new(id, message)
            .with_label(label)
            .with_icon(Icon::named(icon))
            .with_enabled(enabled),
        move |action, _| {
            enz_toolbar_icon(icon, action.message().clone())
                .label(action.label())
                .enabled(action.is_enabled())
                .active(active)
                .id(iced::advanced::widget::Id::new(id))
                .into()
        },
    )
}

/// The updates badge as a pill button in the accent colour.
fn updates_badge_button<'a>(
    icon: &'static str,
    count: usize,
    message: Message,
    label: &str,
) -> Element<'a, Message> {
    use enzui_widgets::focus_button::FocusStatus;
    let body = container(
        iced::widget::row![
            enzui_widgets::icon::enz_icon(icon)
                .size(12.0)
                .tint(iced::Color::WHITE)
                .into_element(),
            enzui_widgets::enz_text::enz_text(count.to_string())
                .size(enzui_widgets::enz_text::TextSize::Caption)
                .weight(iced::font::Weight::Semibold)
                .color(iced::Color::WHITE)
                .announce(false),
        ]
        .spacing(4.0)
        .align_y(iced::Alignment::Center),
    )
    .padding(Padding::from([3.0, 9.0]));
    enzui_widgets::focus_button::focus_button_maybe(body, Some(message))
        .style_focus(|theme: &iced::Theme, status| {
            let p = crate::theme::palette(theme);
            let fill = match status {
                FocusStatus::Hovered => p.accent_hover,
                FocusStatus::Pressed => p.accent_pressed,
                _ => p.accent,
            };
            let focused = matches!(status, FocusStatus::Focused);
            iced::widget::button::Style {
                background: Some(fill.into()),
                text_color: iced::Color::WHITE,
                border: iced::Border {
                    color: if focused {
                        p.focus_ring
                    } else {
                        iced::Color::TRANSPARENT
                    },
                    width: if focused { 1.5 } else { 0.0 },
                    radius: 999.0.into(),
                },
                ..iced::widget::button::Style::default()
            }
        })
        .semantics(
            enzui_core::semantics::Semantics::new(enzui_core::semantics::Role::Button)
                .label(label.to_owned())
                .default_action(),
        )
        .id(iced::advanced::widget::Id::new("store-updates-badge"))
        .into()
}

/// The Repositories button, with a warning dot while a repository did not
/// answer the update check.
fn repositories_action<'a>(warn: bool) -> ActionItem<'a, Message> {
    let id = crate::ui::repositories::REPOSITORIES_INVOKER_ID;
    let label = if warn {
        crate::i18n::label("Repositories: the update check is incomplete")
    } else {
        crate::i18n::label("Manage repositories")
    };
    ActionItem::with_inline(
        Descriptor::new(id),
        Action::new(id, Message::OpenRepositoriesDialog)
            .with_label(label.to_owned())
            .with_icon(Icon::named("server")),
        move |action, _| {
            let button: Element<'a, Message> = enz_toolbar_icon("server", action.message().clone())
                .label(action.label())
                .id(iced::advanced::widget::Id::new(id))
                .into();
            if !warn {
                return button;
            }
            let dot = container(Space::new())
                .width(Length::Fixed(WARNING_DOT_PX))
                .height(Length::Fixed(WARNING_DOT_PX))
                .style(|theme| container::Style {
                    background: Some(crate::theme::palette(theme).warning.into()),
                    border: iced::Border {
                        radius: (WARNING_DOT_PX / 2.0).into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                });
            iced::widget::stack![
                button,
                container(dot)
                    .width(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Right)
                    .padding(iced::Padding::new(0.0).top(3.0).right(3.0)),
            ]
            .into()
        },
    )
}

const WARNING_DOT_PX: f32 = 8.0;

/// The search icon, with the search popup under it while it is open.
fn search_action(store: &crate::Store) -> ActionItem<'_, Message> {
    let id = "store-search-icon";
    let open = store.search_popup_open;
    ActionItem::with_inline(
        Descriptor::new(id),
        Action::new(id, Message::SearchPopupToggled)
            .with_label(crate::i18n::label("Search").to_owned())
            .with_icon(Icon::named("search")),
        move |action, _| {
            let trigger = enz_toolbar_icon("search", action.message().clone())
                .label(action.label())
                .active(open)
                .id(iced::advanced::widget::Id::new(id));
            let content = open.then(|| crate::ui::search_popup::content(store));
            enzui_widgets::popover::enz_popover(trigger, content, || Message::SearchPopupDismissed)
                .width(crate::ui::search_popup::SEARCH_POPUP_WIDTH)
                .label(crate::i18n::label("Search"))
                .scope(iced::advanced::widget::Id::new(
                    crate::ui::search_popup::SEARCH_POPUP_SCOPE,
                ))
                .into_element()
        },
    )
}

/// The shell owns the band and controls; each command can fold independently.
pub(crate) fn titlebar(store: &crate::Store, width: f32) -> Element<'_, Message> {
    let mode = BarMode::for_width(width);
    let mut actions = vec![icon_action(
        "store-refresh",
        "refresh-cw",
        crate::i18n::label("Refresh").to_owned(),
        Message::RefreshRequested,
        !store.is_busy(),
        false,
    )];
    let mut bar = enz_title_bar().tone(enzui_widgets::surface::SurfaceTone::Transparent);
    if crate::ui::shell::shows_back(store) {
        bar = bar.leading(
            enz_toolbar_icon("arrow-left", Message::Back)
                .label(crate::i18n::label("Back"))
                .id(iced::advanced::widget::Id::new("store-back"))
                .into_element(),
        );
    }
    actions.push(search_action(store));
    let badge = store.update_discovery.badge();
    // The badge counts the updates found. An incomplete check is the
    // Repositories button's to report: it marks the repository that did not
    // answer, and its dialog says which.
    let known = match badge {
        crate::services::UpdateBadge::Incomplete { known } => known,
        crate::services::UpdateBadge::Behind(count) => count,
        _ => 0,
    };
    let update_action = (known > 0).then(|| {
        (
            "arrow-down-to-line",
            known,
            crate::i18n::format(
                crate::i18n::localizer(),
                "{update_count} updates available",
                &[("update_count", &known)],
            ),
        )
    });
    if let Some((icon, count, label)) = update_action {
        actions.push(ActionItem::with_inline(
            Descriptor::new("store-updates-badge"),
            Action::new(
                "store-updates-badge",
                Message::LibrarySortChanged(LibrarySort::Updates),
            )
            .with_label(label)
            .with_icon(Icon::named(icon)),
            move |action, _| {
                // The badge is the button: its own fill takes the hover and
                // pressed shades, so the hover tints the badge, not a box
                // around it.
                updates_badge_button(icon, count, action.message().clone(), action.label())
            },
        ));
    }
    actions.push(repositories_action(
        !store.update_discovery.unknown_scopes().is_empty(),
    ));
    if !mode.nav_is_stacked() {
        bar = bar.center(nav_with_activity(
            store.route,
            mode,
            &NavActivity::of(store),
        ));
    }
    bar.actions(
        actions,
        Message::TitlebarOverflow(!store.titlebar_overflow_open),
    )
    .overflow_open(store.titlebar_overflow_open)
    .action_dismiss(Message::TitlebarOverflow(false))
    .action_menu_id_prefix("store-titlebar-actions")
    .action_overflow_trigger_id(iced::advanced::widget::Id::new("store-titlebar-overflow"))
    .into_element()
}

/// Build the navigation band used by the stacked shell layout.
pub(crate) fn stacked_nav_row<'a>(
    route: Route,
    activity: &NavActivity,
    width: f32,
) -> Element<'a, Message> {
    let mode = BarMode::for_width(width);
    debug_assert!(
        mode.nav_is_stacked(),
        "stacked_nav_row is only built at the Stacked width"
    );
    container(nav_with_activity(route, mode, activity))
        .width(Length::Fill)
        .height(Length::Fixed(NAV_ROW_HEIGHT))
        .padding(
            Padding::new(0.0)
                .left(bar_padding_x())
                .right(bar_padding_x())
                .bottom(SpaceToken::Sm.px()),
        )
        .align_y(iced::alignment::Vertical::Center)
        .into()
}
