//! Discover landing page with a featured carousel and responsive collections.
//!
//! Each featured page is a banner in the app's brand colour (Flathub's
//! AppStream `branding`, or an accent wash without one): identity on the
//! left with the Install button under the name and summary, and the top of a
//! screenshot rising from the banner's bottom edge on the right, its top
//! corners rounded. Compact pages place the screenshot below identity while
//! retaining the Install action at the top right. The fixed stage is reserved
//! before image resolution to prevent feed shift.
//!
//! Collection sections exclude featured entries and apps an earlier section
//! already shows, and use responsive grids with
//! borderless Quiet “See All” actions. Expanding a collection replaces the
//! feed in place; Back returns to the section overview.

use crate::model::{self, CollectionKind};
use crate::ui::chrome;
use crate::ui::library::RowDensity;
use crate::{Message, Route, Store};

use enzui_core::paging::PageId;
use enzui_core::semantics::HeadingLevel;
use enzui_theme::spacing::SpaceToken;
use enzui_widgets::banner::{BannerActionEmphasis, BannerTone, enz_banner};
use enzui_widgets::button::{ButtonIconPosition, ButtonKind, ButtonSize, enz_button};
use enzui_widgets::carousel::{CarouselPage, Indicator, body_slot_height, enz_carousel};

use enzui_widgets::enz_pane_header;
use enzui_widgets::enz_text::{TextSize, enz_text};
use enzui_widgets::list_row_surface::{ListRowSurfaceTokens, enz_list_row_surface};
use enzui_widgets::responsive_grid::{CellFn, ColumnRamp, enz_responsive_grid};
use enzui_widgets::scroll_column::enz_scroll_column;
use enzui_widgets::status_page::enz_status_page;
use enzui_widgets::surface::enz_surface;
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length};

const SECTION_ROW_CAP: usize = 8;

/// Share one retry label between saved-catalogue and offline-error surfaces.
pub(crate) const RETRY_LABEL: &str = "Try again";

/// The stale-cache banner's title. Present tense and unambiguous: the store is
/// showing something, and it is not current.
pub(crate) fn stale_banner_title() -> &'static str {
    crate::i18n::label("Showing a saved catalogue")
}

/// The stale-cache banner's body, naming the cache's **age** — "stale" without
/// a number tells a user nothing about whether to trust it.
pub(crate) fn stale_banner_body(age_hours: u64) -> String {
    let age = match age_hours {
        0 => crate::i18n::label("less than an hour ago").to_owned(),
        1 => crate::i18n::label("1 hour ago").to_owned(),
        h if h < 48 => {
            crate::i18n::format(crate::i18n::localizer(), "{h} hours ago", &[("h", &(h))])
        }
        h => crate::i18n::format(
            crate::i18n::localizer(),
            "{} days ago",
            &[("arg0", &(h / 24))],
        ),
    };
    format!(
        "Flathub could not be reached, so these apps come from the copy saved {age}. \
         Newly published apps and updates are missing until it refreshes."
    )
}

/// The title of the nothing-cached-and-offline page. Not "no apps": a failure
/// the user can act on.
pub(crate) fn catalog_error_title() -> &'static str {
    crate::i18n::label("Couldn’t load the Flathub catalogue")
}

/// The body of the nothing-cached-and-offline page, carrying the underlying
/// failure so the user is not told to retry a DNS error forever.
pub(crate) fn catalog_error_description(error: &str) -> String {
    format!(
        "No catalogue has been saved yet, so there is nothing to browse offline. \
         Check your connection and try again. ({error})"
    )
}

// A safe cache-load warning, independent of any simultaneous network error.
fn cache_load_error_banner<'a>(store: &Store) -> Option<Element<'a, Message>> {
    let summary = store.catalog_cache_load_error.as_deref()?;
    Some(
        enz_banner(crate::i18n::text(summary))
            .body(
                crate::i18n::label("The saved catalogue was not used. Fresh catalogue data is shown when available, and the saved data remains protected from replacement."),
            )
            .tone(BannerTone::Danger)
            .dismissible(false)
            .id("discover-catalog-cache-load-error")
            .into_element(),
    )
}

// The stale-catalogue banner, or `None` when the shown catalogue is current.
fn stale_banner<'a>(store: &Store) -> Option<Element<'a, Message>> {
    if !store.catalog_stale {
        return None;
    }
    Some(
        enz_banner(stale_banner_title())
            .body(stale_banner_body(store.catalog_age_hours()))
            .tone(BannerTone::Warning)
            .action_text(crate::i18n::label(RETRY_LABEL))
            .action_emphasis(BannerActionEmphasis::Suggested)
            .on_action(Message::RefreshRequested)
            .dismissible(false)
            .id("discover-catalog-stale")
            .into_element(),
    )
}

/// The bounded featured population, withheld from collection sections to prevent duplicates.
const FEATURED_CAP: usize = 3;

/// Image states share stage height so loading and unavailable screenshots cannot move the feed.
pub(crate) fn featured_stage_height(width: f32, wide: bool) -> f32 {
    if !wide {
        return 400.0;
    }
    let banner = width - 2.0 * SECTION_SIDE_INSET;
    let screenshot = (banner - 2.0 * FEATURED_BANNER_INSET - SpaceToken::Xxl.px()) * 0.6;
    // The carousel's padding, indicator band and the gap above the shot.
    let chrome = 2.0 * CAROUSEL_BODY_PADDING
        + enzui_widgets::carousel::BODY_SPACING
        + FEATURED_INDICATOR.height()
        + FEATURED_SHOT_TOP;
    (screenshot * 0.5 + chrome).round().clamp(340.0, 540.0)
}

/// Shared spacing between independent collection regions, larger than their internal heading gap.
const SECTION_GAP: SpaceToken = SpaceToken::Xl;

// Derive row density from cell width; page width overestimates space in two-column sections.
pub(crate) const TWO_COLUMN_MIN_WIDTH: f32 = 2.0 * 420.0 + 16.0;

pub(crate) const SECTION_SIDE_INSET: f32 = 24.0;

/// How many columns a section grid shows at `content_width`.
pub(crate) fn section_columns(content_width: f32) -> usize {
    if content_width >= TWO_COLUMN_MIN_WIDTH {
        2
    } else {
        1
    }
}

/// The grid ramp for `content_width`: the same column count in every size
/// class, so the breakpoint is this page's own, not the framework's 1024.
pub(crate) fn section_ramp(content_width: f32) -> ColumnRamp {
    let columns = section_columns(content_width);
    ColumnRamp {
        wide: columns,
        medium: columns,
        narrow: columns,
        compact: columns,
    }
}

// The [`RowDensity`] for a row inside a section grid cell at `content_width`.
//
// One column: the cell is the content width. Two columns: the cell is half of
// it, less the column gutter. A row that renders its wide composition into a
// half-width cell is the elision defect this exists to prevent.
pub(crate) fn section_row_density(content_width: f32) -> RowDensity {
    let columns = section_columns(content_width) as f32;
    let gutters = (columns - 1.0) * SpaceToken::Lg.px();
    RowDensity::for_width((content_width - gutters) / columns)
}

const FEATURED_ACTION_WIDTH: f32 = 132.0;

// Featured app names are values; the detail route promotes its app name to a heading.
fn featured_screenshot<'a>(
    entry: &crate::services::AppEntry,
    lead_state: &enzui_core::async_image::ImageState<iced::widget::image::Handle>,
) -> Element<'a, Message> {
    use enzui_core::async_image::ImageState;
    // Only the top corners round: the screenshot rises from the banner's
    // bottom edge, which cuts it off square.
    let top_rounded = iced::border::Radius {
        top_left: FEATURED_SHOT_RADIUS,
        top_right: FEATURED_SHOT_RADIUS,
        bottom_right: 0.0,
        bottom_left: 0.0,
    };
    match lead_state {
        ImageState::Ready(handle @ iced::widget::image::Handle::Rgba { width, height, .. })
            if *width > 0 && *height > 0 =>
        {
            let (image_w, image_h) = (*width, *height);
            let handle = handle.clone();
            iced::widget::responsive(move |size| {
                // The screenshot always keeps its full width: it fills the
                // slot's width and the banner's bottom edge cuts off what
                // does not fit below. One squatter than the slot shows
                // whole, standing on that bottom edge.
                let (slot_w, slot_h) = (size.width.max(1.0), size.height.max(1.0));
                let shown_h = slot_w * image_h as f32 / image_w as f32;
                let visible_h = shown_h.min(slot_h);
                let crop_h = ((visible_h / shown_h) * image_h as f32).round().max(1.0) as u32;
                let shot: Element<'a, Message> = iced::widget::image(handle.clone())
                    .crop(iced::Rectangle {
                        x: 0,
                        y: 0,
                        width: image_w,
                        height: crop_h.min(image_h),
                    })
                    .content_fit(iced::ContentFit::Fill)
                    .filter_method(iced::widget::image::FilterMethod::Linear)
                    .border_radius(top_rounded)
                    .width(Length::Fill)
                    .height(Length::Fixed(visible_h.floor()))
                    .into();
                let slot: Element<'a, Message> = container(shot)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_y(iced::alignment::Vertical::Bottom)
                    .into();
                slot
            })
            .into()
        }
        // Loading, or no screenshot: the same shape in a quiet wash, so the
        // banner keeps its composition.
        _ => {
            let placeholder: Element<'a, Message> = if matches!(lead_state, ImageState::Failed(_)) {
                container(chrome::app_icon(
                    entry.icon_image.as_ref(),
                    &entry.name,
                    FEATURED_ICON_PX * 2.0,
                ))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into()
            } else {
                Space::new().into()
            };
            container(placeholder)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.08).into()),
                    border: iced::Border {
                        radius: top_rounded,
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                })
                .into()
        }
    }
}

// Inset screenshots beyond the carousel navigation controls.
const FEATURED_SHOT_RADIUS: f32 = 12.0;
const FEATURED_SHOT_TOP: f32 = 32.0;
const FEATURED_BANNER_INSET: f32 = 72.0;
const FEATURED_BANNER_RADIUS: f32 = 16.0;

/// The banner's fill: the app's brand colour for the current scheme, or a
/// light accent wash where it declares none.
fn banner_fill(theme: &iced::Theme, brand: Option<([u8; 3], [u8; 3])>) -> iced::Color {
    banner_fill_for(&crate::theme::palette(theme), brand)
}

fn banner_fill_for(
    palette: &enzui_theme::Palette,
    brand: Option<([u8; 3], [u8; 3])>,
) -> iced::Color {
    match brand {
        Some((light, dark)) => {
            let [r, g, b] = if palette.is_dark() { dark } else { light };
            iced::Color::from_rgb8(r, g, b)
        }
        None => {
            let (base, accent) = (palette.surface, palette.accent);
            let mix = |a: f32, b: f32| a + (b - a) * 0.16;
            iced::Color::from_rgb(
                mix(base.r, accent.r),
                mix(base.g, accent.g),
                mix(base.b, accent.b),
            )
        }
    }
}

/// Text that reads on the banner, at full or secondary strength.
fn banner_ink(
    theme: &iced::Theme,
    brand: Option<([u8; 3], [u8; 3])>,
    strength: f32,
) -> iced::Color {
    let ink = enzui_theme::Palette::ink_on(banner_fill(theme, brand));
    iced::Color { a: strength, ..ink }
}

/// The banner's Install control: a button in the banner's ink with the
/// label in the banner's own colour, so it belongs to every brand colour.
fn banner_install<'a>(
    entry: &crate::services::AppEntry,
    brand: Option<([u8; 3], [u8; 3])>,
    palette: &enzui_theme::Palette,
) -> Element<'a, Message> {
    let fill = banner_fill_for(palette, brand);
    let ink = enzui_theme::Palette::ink_on(fill);
    let toward_fill = |amount: f32| iced::Color {
        r: ink.r + (fill.r - ink.r) * amount,
        g: ink.g + (fill.g - ink.g) * amount,
        b: ink.b + (fill.b - ink.b) * amount,
        a: 1.0,
    };
    let mut tokens = enzui_widgets::tokens::ButtonTokens::from_palette(palette);
    tokens.primary_bg = ink;
    tokens.primary_hover_bg = toward_fill(0.14);
    tokens.primary_pressed_bg = toward_fill(0.26);
    tokens.primary_fg = fill;
    tokens.primary_border = iced::Color::TRANSPARENT;
    tokens.primary_border_width = 0.0;
    enz_button(
        crate::i18n::label("Install"),
        Message::InstallRequested(entry.id.clone(), entry.name.clone()),
    )
    .kind(ButtonKind::Primary)
    .size(ButtonSize::Lg)
    .icon("download")
    .icon_position(ButtonIconPosition::Leading)
    .tokens(tokens)
    .aria_label(crate::i18n::format(
        crate::i18n::localizer(),
        "Install {}",
        &[("arg0", &entry.name)],
    ))
    .into_element()
}

/// A label with its icon, in the banner's ink.
fn banner_fact<'a>(
    icon: &'static str,
    label: String,
    brand: Option<([u8; 3], [u8; 3])>,
) -> Element<'a, Message> {
    row![
        enzui_widgets::icon::enz_icon(icon)
            .size(14.0)
            .tint_resolved(move |t| banner_ink(t, brand, 0.8))
            .into_element(),
        enz_text(label)
            .size(TextSize::Footnote)
            .color_resolved(move |t| banner_ink(t, brand, 0.8)),
    ]
    .spacing(SpaceToken::Xs.px())
    .align_y(Alignment::Center)
    .into()
}

fn featured_panel<'a>(
    store: &'a Store,
    entry: &crate::services::AppEntry,
    density: RowDensity,
    lead_state: enzui_core::async_image::ImageState<iced::widget::image::Handle>,
    brand: Option<([u8; 3], [u8; 3])>,
    stage_height: f32,
) -> Element<'a, Message> {
    let palette = &store.theme_ref().palette;
    let operation = store.operation_for(&entry.id);
    let repositories = store.repository_answer();
    let name = enz_text(entry.name.clone())
        .size(TextSize::Title1)
        .weight(iced::font::Weight::Bold)
        .single_line_elided()
        .full_width()
        .color_resolved(move |t| banner_ink(t, brand, 1.0));

    let mut facts = row![]
        .spacing(SpaceToken::Lg.px())
        .align_y(Alignment::Center);
    if entry.verified {
        facts = facts.push(banner_fact(
            "badge-check",
            crate::i18n::label("Verified").to_owned(),
            brand,
        ));
    }
    let installs = model::format_installs(entry.downloads);
    if !installs.is_empty() {
        facts = facts.push(banner_fact("download", installs, brand));
    }

    // The one explicit control on the featured surface: the surface itself
    // activates the detail route (`enz_list_row_surface` below), and yields to
    // this button via `focus_button::is_event_captured`.
    let install: Element<Message> =
        if let Some(op) = operation.filter(|op| !(op.finished && op.error.is_none())) {
            // Starting the install does not navigate, so its progress has to be
            // visible in the slot the button occupied.
            chrome::operation_slot(op, Length::Fixed(FEATURED_ACTION_WIDTH), true)
        } else if entry.installed {
            banner_fact("check", crate::i18n::label("Installed").to_owned(), brand)
        } else {
            // Featured installation requires a source repository this build can mutate.
            match repositories.authorization(&entry.origin).refusal() {
                None => banner_install(entry, brand, palette),
                Some(refusal) => banner_fact("lock", refusal.label.to_owned(), brand),
            }
        };

    let icon_px = if density.shows_installs() {
        FEATURED_ICON_PX
    } else {
        chrome::ROW_ICON_PX
    };
    // The icon with the action beside it, then the name, the summary and
    // the facts, each on a line of its own.
    let wide = density.shows_installs();
    let mut install = Some(install);
    let lead: Element<'a, Message> = if wide {
        row![
            chrome::app_icon(entry.icon_image.as_ref(), &entry.name, icon_px),
            install.take().unwrap_or_else(|| Space::new().into()),
        ]
        .spacing(SpaceToken::Lg.px())
        .align_y(Alignment::Center)
        .into()
    } else {
        chrome::app_icon(entry.icon_image.as_ref(), &entry.name, icon_px)
    };
    let mut details = column![lead, name].spacing(SpaceToken::Sm.px());
    if !entry.summary.is_empty() {
        details = details.push(
            enz_text(entry.summary.clone())
                .size(TextSize::Body)
                .color_resolved(move |t| banner_ink(t, brand, 0.8)),
        );
    }

    let screenshot = || {
        container(featured_screenshot(entry, &lead_state))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::new(0.0).top(FEATURED_SHOT_TOP))
    };
    // Wide: identity on the left, the screenshot rising from the banner's
    // bottom edge on the right. Compact: identity on top, the screenshot
    // rising under it.
    let content: Element<'a, Message> = if wide {
        details = details
            .push(container(facts).padding(iced::Padding::new(0.0).top(SpaceToken::Xs.px())));
        row![
            container(details.width(Length::Fill))
                .width(Length::FillPortion(4))
                .height(Length::Fill)
                .align_y(iced::alignment::Vertical::Center),
            container(screenshot()).width(Length::FillPortion(6)),
        ]
        .spacing(SpaceToken::Xxl.px())
        .padding(
            iced::Padding::new(0.0)
                .left(FEATURED_BANNER_INSET)
                .right(FEATURED_BANNER_INSET),
        )
        .into()
    } else {
        column![
            row![
                details.width(Length::Fill),
                install.take().unwrap_or_else(|| Space::new().into())
            ]
            .spacing(SpaceToken::Md.px())
            .width(Length::Fill),
            screenshot(),
        ]
        .padding(
            iced::Padding::new(0.0)
                .top(SpaceToken::Lg.px())
                .left(FEATURED_BANNER_INSET)
                .right(FEATURED_BANNER_INSET),
        )
        .into()
    };

    let banner = enz_surface(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .radius(FEATURED_BANNER_RADIUS)
        .clip(true)
        .fill_resolved(move |t| banner_fill(t, brand))
        .into_element();

    // The whole constant-height stage activates Detail while nested controls keep their events; decoded images must not move the feed.
    let detail_msg = Message::open_app_detail(entry);
    let panel = container(banner).width(Length::Fill).height(Length::Fill);
    enz_list_row_surface(panel, detail_msg)
        .tokens_resolved(|theme| ListRowSurfaceTokens {
            hover_overlay: iced::Color::TRANSPARENT,
            ..ListRowSurfaceTokens::from_palette(&crate::theme::palette(theme))
        })
        .content_padding(0.0)
        .label(crate::i18n::format(
            crate::i18n::localizer(),
            "Featured: {}",
            &[("arg0", &(entry.name))],
        ))
        .row_height(body_slot_height(stage_height, FEATURED_INDICATOR))
        .width(Length::Fill)
        .into_element()
}

/// The carousel's page indicator; the panel's height is computed for it.
const FEATURED_INDICATOR: Indicator = Indicator::Lines;

const FEATURED_ICON_PX: f32 = 64.0;

// App IDs preserve featured selection across catalog reordering.
fn featured_carousel<'a>(
    store: &'a Store,
    entries: &'a [crate::services::AppEntry],
    selected: Option<PageId>,
    density: RowDensity,
    width: f32,
) -> Element<'a, Message> {
    let stage_height = featured_stage_height(width, density.shows_installs());
    let pages: Vec<CarouselPage<'a, Message>> = entries
        .iter()
        .map(|entry| {
            let lead_state = store.featured_image_state(&entry.id);
            let brand = store.featured_brand(&entry.id);
            CarouselPage::new(entry.id.clone(), move || {
                featured_panel(
                    store,
                    entry,
                    density,
                    lead_state.clone(),
                    brand,
                    stage_height,
                )
            })
        })
        .collect();

    enz_carousel(pages, selected, Message::FeaturedPageSelected)
        .label(crate::i18n::label("Featured apps"))
        .show_navigation(entries.len() > 1)
        .indicator(FEATURED_INDICATOR)
        // Past the last banner comes the first again.
        .wrap(enzui_core::paging::Wrap::Wrap)
        // The banner runs under the arrows; its own inset clears them.
        .chrome_inset(false)
        .swipe_enabled(true)
        .height(Length::Fixed(stage_height))
        .width(Length::Fill)
        .into()
}

// A compact app row for the Discover sections — reuses the search result row,
// which is the port's `CompactAppRow` (`components.slint:277`) equivalent
// (icon + name + one-line summary + trailing action).
fn discover_row<'a>(
    entry: &crate::services::AppEntry,
    density: RowDensity,
    operations: &[crate::model::OperationState],
    id_prefix: String,
    repositories: crate::services::RepositoryAnswer<'_>,
) -> Element<'a, Message> {
    crate::ui::search::discover_result_row(
        entry,
        density,
        crate::operation_for_in(operations, &entry.id),
        Some(id_prefix),
        repositories,
    )
}

// Exclude featured and earlier section entries so Discover never repeats an app.
fn collection_section<'a>(
    kind: CollectionKind,
    // Rows clone displayed fields, so returned elements must not borrow a temporary candidate list.
    items: &'a [crate::services::AppEntry],
    exclude: &mut Vec<&'a str>,
    density: RowDensity,
    ramp: ColumnRamp,
    operations: &'a [crate::model::OperationState],
    // Resolve installation authority per origin using both repository state and rows.
    repositories: crate::services::RepositoryAnswer<'a>,
) -> Option<Element<'a, Message>> {
    // The page shows at most three featured and 4 x 8 section ids, so a
    // linear exclusion is cheaper than a per-frame set allocation.
    let rows: Vec<&'a crate::services::AppEntry> = items
        .iter()
        .filter(|e| !exclude.contains(&e.id.as_str()))
        .take(SECTION_ROW_CAP)
        .collect();
    if rows.is_empty() {
        return None;
    }
    // Later sections skip what this one shows: Trending and Popular overlap
    // heavily in practice, and the same app twice on one page reads as filler.
    exclude.extend(rows.iter().map(|e| e.id.as_str()));

    // Derive section heading, accessible destination and action from the same collection identity.
    let heading = kind.heading();
    let action_id = match kind {
        CollectionKind::Trending => "store-see-all-trending",
        CollectionKind::Popular => "store-see-all-popular",
        CollectionKind::RecentlyAdded => "store-see-all-recently-added",
        CollectionKind::RecentlyUpdated => "store-see-all-recently-updated",
    };
    let header = enz_pane_header(heading.to_owned())
        .title_size(TextSize::Title2.px())
        .heading(HeadingLevel::Section)
        .trailing(
            enz_button(
                crate::i18n::label("See All"),
                Message::ExpandCollection(kind),
            )
            .kind(ButtonKind::Quiet)
            .id(iced::advanced::widget::Id::new(action_id))
            .size(ButtonSize::Sm)
            .icon("chevron-right")
            .icon_position(ButtonIconPosition::Trailing)
            .aria_label(crate::i18n::format(
                crate::i18n::localizer(),
                "See all {heading}",
                &[("heading", &(heading))],
            ))
            .into(),
        );

    let grid = app_grid(heading, rows, density, ramp, operations, repositories);
    Some(
        column![header.into_element(), grid]
            .spacing(SpaceToken::Sm.px())
            .width(Length::Fill)
            .into(),
    )
}

fn app_grid<'a>(
    section: &str,
    rows: impl IntoIterator<Item = &'a crate::services::AppEntry>,
    density: RowDensity,
    ramp: ColumnRamp,
    operations: &'a [crate::model::OperationState],
    repositories: crate::services::RepositoryAnswer<'a>,
) -> Element<'a, Message> {
    let cells: Vec<CellFn<'a, Message>> = rows
        .into_iter()
        .map(|entry| {
            let id_prefix = format!("store-discover-{section}-{}", entry.id);
            let row: CellFn<'a, Message> = Box::new(move || {
                discover_row(entry, density, operations, id_prefix.clone(), repositories)
            });
            row
        })
        .collect();
    enz_responsive_grid(cells)
        .ramp(ramp)
        .gap_x(SpaceToken::Lg)
        .gap_y(SpaceToken::Xs)
        .into()
}

// Reuse the cached ranking instead of sorting the catalog in each view.
fn featured_entries(store: &Store) -> &[crate::services::AppEntry] {
    let popular = store.popular_rows();
    let source = if !popular.is_empty() {
        popular
    } else if !store.catalog.trending.is_empty() {
        &store.catalog.trending
    } else {
        &store.catalog.new
    };
    &source[..source.len().min(FEATURED_CAP)]
}

// Expanded collections are unfiltered because the featured feed is no longer visible.
fn expanded_collection_view<'a>(
    store: &'a Store,
    kind: CollectionKind,
    width: f32,
) -> Element<'a, Message> {
    let density = RowDensity::for_width(width - 32.0);
    let heading = kind.heading();
    let rows = kind.rows(&store.catalog, &store.recently_uninstalled);

    let header = enz_pane_header(heading.to_owned())
        .title_size(TextSize::Title2.px())
        .heading(HeadingLevel::Section);

    let mut body: Vec<Element<Message>> = vec![header.into_element()];
    if rows.is_empty() {
        body.push(
            container(
                enz_status_page(crate::i18n::format(
                    crate::i18n::localizer(),
                    "No {} apps yet.",
                    &[("arg0", &(heading.to_ascii_lowercase()))],
                ))
                .icon("package"),
            )
            .width(Length::Fill)
            .into(),
        );
    } else {
        for entry in &rows {
            body.push(discover_row(
                entry,
                density,
                &store.operations,
                format!("store-expanded-{}-{}", kind.heading(), entry.id),
                store.repository_answer(),
            ));
        }
    }

    enz_scroll_column(body)
        .inset(chrome::page_inset())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Show an expanded collection or featured sections, distinguishing loading, empty and saved-catalogue states.
pub(crate) fn view<'a>(store: &'a Store, width: f32) -> Element<'a, Message> {
    if let Some(kind) = store.expanded_collection {
        return expanded_collection_view(store, kind, width);
    }

    // Page and cell density account for the shared scroll insets at different composition boundaries.
    let density = RowDensity::for_width(width - 32.0);
    // The sections' content width.
    let section_width = width - 2.0 * SECTION_SIDE_INSET;
    let cell_density = section_row_density(section_width);
    let ramp = section_ramp(section_width);

    // The complete index establishes content even when every ranking rail is unavailable.
    let official = store.enzimos_home_entries();
    let has_content = !official.is_empty()
        || !store.catalog_index.is_empty()
        || !store.catalog.popular.is_empty()
        || !store.catalog.trending.is_empty()
        || !store.catalog.new.is_empty();

    if !has_content {
        let page: Element<Message> = if store.loading_catalog {
            // Loading remains distinct from a successfully resolved empty catalogue.
            enz_status_page(crate::i18n::label("Loading Flathub catalog…"))
                .icon("loader")
                .description(crate::i18n::label("Fetching app metadata from Flathub."))
                .into()
        } else if let Some(err) = &store.catalog_error {
            // An empty cache and a failed fetch is an actionable failure, not
            // "no apps exist". It uses the same assertive Danger alert as the
            // other visible failures while retaining both recovery actions.
            column![
                super::chrome::failure_banner(
                    catalog_error_title(),
                    Some(catalog_error_description(err)),
                )
                .into_element(),
                row![
                    enz_button(crate::i18n::label(RETRY_LABEL), Message::RefreshRequested)
                        .kind(ButtonKind::Accent)
                        .icon("refresh-cw"),
                    enz_button(
                        crate::i18n::label("Open Library"),
                        Message::Navigate(Route::Library)
                    )
                    .kind(ButtonKind::Ghost),
                ]
                .spacing(SpaceToken::Sm.px()),
            ]
            .spacing(SpaceToken::Md.px())
            .width(Length::Fill)
            .into()
        } else {
            enz_status_page(crate::i18n::label("No apps available yet."))
                .icon("compass")
                .description(
                    "Connect to a network and refresh, or add a flatpak remote \
                     under Library → Repositories.",
                )
                .primary_action(enzui_core::action::Action::new(
                    crate::i18n::label("Open Library"),
                    Message::Navigate(Route::Library),
                ))
                .into()
        };
        let page = if let Some(banner) = cache_load_error_banner(store) {
            column![banner, page]
                .spacing(SpaceToken::Md.px())
                .width(Length::Fill)
                .into()
        } else {
            page
        };
        return container(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }

    let mut body: Vec<Element<Message>> = Vec::new();

    // A persistence warning remains visible even when a network refresh
    // succeeds; it is not the same condition as a stale or failed ingest.
    if let Some(banner) = cache_load_error_banner(store) {
        body.push(banner);
        body.push(Space::new().height(Length::Fixed(SECTION_GAP.px())).into());
    }

    // A usable offline cache must disclose its saved age and provide the actual retry action.
    if let Some(banner) = stale_banner(store) {
        body.push(banner);
        body.push(Space::new().height(Length::Fixed(SECTION_GAP.px())).into());
    }

    let featured = featured_entries(store);
    let mut carousel_index = None;
    if !featured.is_empty() {
        carousel_index = Some(body.len());
        body.push(featured_carousel(
            store,
            featured,
            store.featured_page.clone(),
            density,
            width,
        ));
        // The carousel's own bottom padding already parts it from the first
        // section; a full section gap on top of it left an empty band.
        body.push(
            Space::new()
                .height(Length::Fixed(SpaceToken::Sm.px()))
                .into(),
        );
    }
    let mut shown_ids: Vec<&str> = featured.iter().map(|e| e.id.as_str()).collect();

    // Use the state-bound sorted projection so layout does not rerun catalogue sorting.
    let popular = store.popular_rows();
    let sections: [(CollectionKind, &[crate::services::AppEntry]); 4] = [
        (CollectionKind::Trending, &store.catalog.trending),
        (CollectionKind::Popular, popular),
        (CollectionKind::RecentlyAdded, &store.catalog.new),
        (CollectionKind::RecentlyUpdated, &store.catalog.updated),
    ];
    let official: Vec<_> = official
        .into_iter()
        .filter(|entry| !shown_ids.contains(&entry.id.as_str()))
        .collect();
    let mut first_section = official.is_empty();
    if !first_section {
        let grid = app_grid(
            "enzimos",
            official.iter().copied(),
            cell_density,
            ramp,
            &store.operations,
            store.repository_answer(),
        );
        shown_ids.extend(official.iter().map(|entry| entry.id.as_str()));
        body.push(
            column![
                enz_pane_header("enzim Apps".to_owned())
                    .title_size(TextSize::Title2.px())
                    .heading(HeadingLevel::Section)
                    .into_element(),
                grid,
            ]
            .spacing(SpaceToken::Sm.px())
            .into(),
        );
    }
    for (kind, items) in sections {
        if let Some(section) = collection_section(
            kind,
            items,
            &mut shown_ids,
            cell_density,
            ramp,
            &store.operations,
            store.repository_answer(),
        ) {
            if !first_section {
                body.push(Space::new().height(Length::Fixed(SECTION_GAP.px())).into());
            }
            body.push(section);
            first_section = false;
        }
    }

    // The carousel pads its stage by its own body padding, so the page
    // insets everything else by that much more: the banner then lines up
    // with the sections' edges, and sits closer under the titlebar.
    let outer = (chrome::page_inset() - CAROUSEL_BODY_PADDING).max(0.0);
    let body: Vec<Element<Message>> = body
        .into_iter()
        .enumerate()
        .map(|(index, element)| {
            if Some(index) == carousel_index {
                element
            } else {
                container(element)
                    .padding(
                        iced::Padding::new(0.0)
                            .left(CAROUSEL_BODY_PADDING)
                            .right(CAROUSEL_BODY_PADDING),
                    )
                    .width(Length::Fill)
                    .into()
            }
        })
        .collect();
    enz_scroll_column(body)
        .inset(outer)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// The carousel's padding around its pages.
const CAROUSEL_BODY_PADDING: f32 = enzui_widgets::carousel::BODY_PADDING;
