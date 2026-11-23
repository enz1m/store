//! Search and Categories.
//!
//! Search matches **names and descriptions** (the user's explicit ask in
//! #2 \[46\]) via `model::relevance_score`, with icon+label filter chips rather
//! than filled badges. Results go through the same `virtual_list` the Library
//! uses, so the 10k+ Flathub catalogue is browsable rather than sliced 8 at a
//! time as round one did.

use crate::model::{self, BROWSE_CATEGORIES, OperationState};
use crate::services::AppEntry;
use crate::ui::chrome::{self, ChipTone};
use crate::ui::library::RowDensity;
use crate::{Message, Store};

use enzui_theme::SizingTokens;
use enzui_theme::spacing::SpaceToken;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button, enz_filter_chip};
use enzui_widgets::clickable::enz_clickable;
use enzui_widgets::enz_text::{TextSize, enz_text};
use enzui_widgets::flow::enz_flow;

use enzui_widgets::scroll_column::enz_scroll_column;
use enzui_widgets::search_field::enz_search_field;
use enzui_widgets::status_page::enz_status_page;
use enzui_widgets::virtual_list::enz_virtual_list;
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Padding};

fn filter_chip<'a>(
    icon: &'static str,
    label: &'static str,
    active: bool,
    aria: &str,
    msg: Message,
) -> Element<'a, Message> {
    enz_filter_chip(label, active, msg)
        .icon(icon)
        .aria_label(aria)
        .into_element()
}

pub(crate) fn recent_searches_section<'a>(history: &[String]) -> Element<'a, Message> {
    let heading = enz_text(crate::i18n::label("Recent Searches"))
        .size(TextSize::Footnote)
        .weight(iced::font::Weight::Semibold)
        .color_resolved(|t| crate::theme::palette(t).text_muted);
    let mut col = column![heading]
        .spacing(SpaceToken::Xs.px())
        .width(Length::Fill);
    for query in history {
        let body = row![
            enzui_widgets::icon::enz_icon("clock")
                .size(SizingTokens::default().icon_xs)
                .tint_resolved(|t| crate::theme::palette(t).text_muted),
            enz_text(query.clone())
                .size(TextSize::Body)
                .color_resolved(|t| crate::theme::palette(t).text),
        ]
        .spacing(SpaceToken::Sm.px())
        .align_y(Alignment::Center);
        // Resolve the pointer target floor from the framework token rather than a coincidentally equal literal.
        let hit_h = SizingTokens::default().touch_target;
        col = col.push(
            enz_clickable(body, Message::RecentSearchPicked(query.clone()))
                .aria_label(crate::i18n::format(
                    crate::i18n::localizer(),
                    "Search for {query}",
                    &[("query", &(query))],
                ))
                .width(Length::Fill)
                .height(Length::Fixed(hit_h))
                .radius(6.0)
                // Clickable content centres by default; search suggestions use list alignment.
                .align_x(iced::Alignment::Start)
                .align_y(iced::Alignment::Center)
                .padding(SpaceToken::Sm.px())
                .into_element(),
        );
    }
    col.into()
}

// Path-free note shown when the bounded read pool refuses a recent-search
// reload; retrying resubmits it.
pub(crate) const RECENT_SEARCHES_BUSY_MESSAGE: &str = "The system was busy; try again.";

/// The recovery strip for the recent-search history: a path-free banner with
/// Retry and Retire actions when the file is refused whole, the transient
/// over-long-query refusal, and a note when a reload is refused by the read
/// pool. `None` when none of these is set.
pub(crate) fn recent_searches_recovery<'a>(store: &Store) -> Option<Element<'a, Message>> {
    let mut strip = column![].spacing(SpaceToken::Sm.px()).width(Length::Fill);
    let mut any = false;
    if store.recent_searches_blocked() {
        any = true;
        let message = store.recent_searches_unavailable.map_or(
            crate::i18n::label("Recent searches are paused."),
            |reason| reason.user_summary(),
        );
        let retry: Element<Message> = if store.recent_searches_loading {
            enz_text(crate::i18n::label("Checking\u{2026}"))
                .size(TextSize::Footnote)
                .color_resolved(|t| crate::theme::palette(t).text_muted)
                .into()
        } else {
            enz_button(crate::i18n::label("Retry"), Message::RecentSearchesRetry)
                .kind(ButtonKind::Quiet)
                .size(ButtonSize::Sm)
                .into()
        };
        let actions = row![
            retry,
            enz_button(crate::i18n::label("Retire"), Message::RecentSearchesRetired)
                .kind(ButtonKind::Plain)
                .size(ButtonSize::Sm),
        ]
        .spacing(SpaceToken::Sm.px());
        strip = strip.push(
            column![
                enz_text(message)
                    .size(TextSize::Body)
                    .color_resolved(|t| crate::theme::palette(t).text),
                actions,
            ]
            .spacing(SpaceToken::Xs.px()),
        );
    }
    if store.recent_searches_read_busy {
        any = true;
        strip = strip.push(
            enz_text(crate::i18n::label(RECENT_SEARCHES_BUSY_MESSAGE))
                .size(TextSize::Footnote)
                .color_resolved(|t| crate::theme::palette(t).text_muted),
        );
    }
    if let Some(message) = &store.recent_searches_save_error {
        any = true;
        strip = strip.push(
            enz_text(message.clone())
                .size(TextSize::Footnote)
                .color_resolved(|t| crate::theme::palette(t).text_muted),
        );
    }
    if let Some(refusal) = &store.recent_search_refusal {
        any = true;
        strip = strip.push(
            enz_text(refusal.user_summary())
                .size(TextSize::Footnote)
                .color_resolved(|t| crate::theme::palette(t).text_muted),
        );
    }
    any.then(|| strip.into())
}

pub(crate) const ROW_PROGRESS_WIDTH: f32 = 118.0;

// Pass repository state with its rows so unavailable authority cannot look like an empty catalog.
pub(crate) fn result_row_with_operation<'a>(
    entry: &AppEntry,
    density: RowDensity,
    operation: Option<&OperationState>,
    id_prefix: Option<String>,
    repositories: crate::services::RepositoryAnswer<'_>,
) -> Element<'a, Message> {
    catalogue_row(entry, density, operation, id_prefix, repositories)
}

pub(crate) fn discover_result_row<'a>(
    entry: &AppEntry,
    density: RowDensity,
    operation: Option<&OperationState>,
    id_prefix: Option<String>,
    repositories: crate::services::RepositoryAnswer<'_>,
) -> Element<'a, Message> {
    catalogue_row(entry, density, operation, id_prefix, repositories)
}

fn catalogue_row<'a>(
    entry: &AppEntry,
    density: RowDensity,
    operation: Option<&OperationState>,
    id_prefix: Option<String>,
    repositories: crate::services::RepositoryAnswer<'_>,
) -> Element<'a, Message> {
    // Shared subtitle construction retains the empty-metadata fallback within fixed-height narrow rows.
    let installs = if density.shows_installs() {
        model::format_installs(entry.downloads)
    } else {
        String::new()
    };
    // Installation state belongs only to the trailing slot.
    let subtitle: Element<Message> = chrome::AppRowSubtitle::<Message>::new(entry.id.clone())
        .summary(entry.summary.clone())
        .stack(density.shows_installs())
        .push_chip_if(entry.verified, || {
            chrome::icon_chip(
                "badge-check",
                crate::i18n::label("Verified"),
                ChipTone::Success,
            )
        })
        .push_chip_if(!installs.is_empty(), || {
            chrome::icon_chip("download", installs.clone(), ChipTone::Muted)
        })
        .build();

    let trailing: Element<Message> = if let Some(op) =
        operation.filter(|op| op.app_id == entry.id && !(op.finished && op.error.is_none()))
    {
        // A successfully finished operation is excluded: the reloaded
        // installed list already turns this row into an installed one, and an
        // operation-derived chip would double-announce it.
        chrome::operation_slot(
            op,
            Length::Fixed(ROW_PROGRESS_WIDTH),
            density.shows_action_labels(),
        )
    } else if entry.installed {
        // The check alone, where the Install glyph sits on the rows around
        // it: centred in a box the button's size. Every catalogue list, at
        // every width, uses the same two glyphs.
        let chip = chrome::state_chip(
            "check",
            crate::i18n::label("Installed"),
            ChipTone::Success,
            false,
        );
        let button =
            enzui_widgets::toolbar_icon::TitlebarAffordanceTokens::DEFAULT.effective_control_size();
        container(chip)
            .center_x(Length::Fixed(button))
            .center_y(Length::Fixed(button))
            .into()
    } else {
        // Offer Install only with authoritative writable source-repository evidence.
        // The glyph button on every list; the app page's filled Install
        // stays the one primary call to action.
        match repositories.authorization(&entry.origin).refusal() {
            None => chrome::discover_install(entry),
            Some(refusal) => crate::ui::library::refusal_chip(refusal),
        }
    };
    // Keep every trailing state clear of the scrollbar gutter.
    let trailing: Element<Message> = container(trailing)
        .padding(Padding::new(0.0).right(SpaceToken::Sm.px()))
        .into();

    chrome::app_row(
        entry.icon_image.as_ref(),
        &entry.name,
        subtitle,
        trailing,
        Message::open_app_detail(entry),
        id_prefix,
    )
}

// Resolve caption contrast against the page surface, not a tile background.
fn category_section_count_ink(theme: &iced::Theme) -> iced::Color {
    let p = crate::theme::palette(theme);
    p.text_subtle_on(p.surface)
}

/// The in-page search field's widget id (the Ctrl+F target on this page).
pub(crate) const INLINE_SEARCH_FIELD_ID: &str = "store-search-inline";

/// The Search view.
pub(crate) fn view<'a>(store: &'a Store, rows: &'a [AppEntry], width: f32) -> Element<'a, Message> {
    let density = RowDensity::for_width(width - 32.0);

    // The titlebar has only a search icon, so the page carries the field.
    let mut head: Vec<Element<Message>> = Vec::new();
    {
        head.push(
            enz_search_field(&store.search_query, Message::SearchInput)
                .placeholder(crate::i18n::text("Search apps and descriptions…"))
                .on_clear(Message::SearchCleared)
                .on_submit(Message::SearchSubmitted)
                .width(Length::Fill)
                // Mount-time focus applies once so subsequent redraws cannot steal an active caret.
                .focus_on_mount(true)
                // Stable id kept for test assertions.
                .id(iced::advanced::widget::Id::new(INLINE_SEARCH_FIELD_ID))
                .into(),
        );

        head.push(
            Space::new()
                .height(Length::Fixed(SpaceToken::Space5.px()))
                .into(),
        );
    }

    // A wrapping flow, not a Row: at 375px four chips do not fit on one line
    // and a Row would clip the last one mid-glyph rather than wrap it.
    let f = store.filters;
    let chips = enz_flow()
        .gap(SpaceToken::Sm)
        .cross_gap(SpaceToken::Sm)
        .align_items(Alignment::Center)
        .fill(true)
        .push(filter_chip(
            "badge-check",
            crate::i18n::label("Verified"),
            f.verified_only,
            model::verified_filter_aria_label(f.verified_only),
            Message::ToggleVerifiedOnly,
        ))
        .push(filter_chip(
            "scale",
            crate::i18n::label("Open source"),
            f.foss_only,
            model::foss_filter_aria_label(f.foss_only),
            Message::ToggleFossOnly,
        ))
        .push(filter_chip(
            "circle-slash",
            crate::i18n::label("Hide EOL"),
            f.hide_eol,
            model::hide_eol_filter_aria_label(f.hide_eol),
            Message::ToggleHideEol,
        ))
        .push(filter_chip(
            "check",
            crate::i18n::label("Installed"),
            f.installed_only,
            model::installed_filter_aria_label(f.installed_only),
            Message::ToggleInstalledOnly,
        ));

    head.push(chips.into_element());

    head.push(
        Space::new()
            .height(Length::Fixed(SpaceToken::Md.px()))
            .into(),
    );

    let count_line = if store.search_query.trim().is_empty() {
        String::new()
    } else {
        format!(
            "{} result{}",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        )
    };
    if !count_line.is_empty() {
        head.push(
            row![
                chrome::section_label(count_line),
                Space::new().width(Length::Fill),
            ]
            .width(Length::Fill)
            .into(),
        );

        head.push(
            Space::new()
                .height(Length::Fixed(SpaceToken::Md.px()))
                .into(),
        );
    }

    let head_block = column(head).width(Length::Fill);

    // Preserve the field ancestor chain when the first typed character changes the result body.
    // The filter strip shares the result rows' readable measure.
    let head_region = enz_scroll_column(vec![chrome::readable_measure(head_block)])
        .insets(
            SpaceToken::Lg.px(),
            SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER,
            SpaceToken::Lg.px(),
            0.0,
        )
        .spacing(SpaceToken::None.px())
        .width(Length::Fill)
        .height(Length::Shrink);

    let head_region: Element<Message> = match recent_searches_recovery(store) {
        Some(banner) => column![
            head_region,
            container(chrome::readable_measure(banner))
                .padding(Padding::from([0.0, SpaceToken::Lg.px()]))
                .width(Length::Fill),
        ]
        .width(Length::Fill)
        .into(),
        None => head_region.into(),
    };

    if rows.is_empty() {
        if store.search_query.trim().is_empty() && !store.recent_searches.is_empty() {
            let body = enz_scroll_column(vec![chrome::readable_measure(recent_searches_section(
                &store.recent_searches,
            ))])
            .insets(
                SpaceToken::Lg.px(),
                SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER,
                0.0,
                SpaceToken::Lg.px(),
            )
            .width(Length::Fill)
            .height(Length::Fill);
            return column![head_region, body]
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }
        let page = if store.search_query.trim().is_empty() {
            enz_status_page(crate::i18n::label("Search apps"))
                .icon("search")
                .description(crate::i18n::label(
                    "Type to search configured repositories.",
                ))
        } else if store.loading_catalog {
            enz_status_page(crate::i18n::label("Loading catalogue…"))
                .icon("loader")
                .description(crate::i18n::label("Fetching app metadata from Flathub."))
        } else {
            enz_status_page(crate::i18n::format(
                crate::i18n::localizer(),
                "No results for “{}”",
                &[("arg0", &(store.search_query.trim()))],
            ))
            .icon("search-x")
            .description(if f.any() {
                crate::i18n::label("Try clearing the filters above.")
            } else {
                crate::i18n::label("Try a shorter or differently-spelled query.")
            })
        };
        let body = enz_scroll_column(vec![container(page).width(Length::Fill).into()])
            .insets(
                SpaceToken::Lg.px(),
                SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER,
                0.0,
                SpaceToken::Lg.px(),
            )
            .width(Length::Fill)
            .height(Length::Fill);
        return column![head_region, body]
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }

    let provider = store.row_provider(rows);
    let operations = &store.operations;
    let repositories = store.repository_answer();
    // The published viewport geometry, kept current by `SearchScrolled`.
    let geometry = store.search_scroll.clone();
    let live = geometry.get();

    let list: Element<Message> =
        // Row padding and virtual pitch must use the same gap to avoid realization drift.
        enz_virtual_list(provider, chrome::row_pitch(), move |_i, entry| {

            container(result_row_with_operation(
                entry,
                density,
                crate::operation_for_in(operations, &entry.id),
                None,
                repositories,
            ))
            .padding(
                    Padding::new(0.0)
                        // The page's trailing inset lives in each row, so the
                        // list (and its scrollbar) reaches the window edge.
                        .right(SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER)
                        .bottom(chrome::ROW_GAP.px()),
                )
                .width(Length::Fill)
                .into()
        })
        .viewport_height(live.map_or(crate::ui::library::FIRST_FRAME_VIEWPORT, |g| {
            g.bounds.height
        }))
        .scroll_offset(live.map_or(0.0, |g| g.translation.y))
        .overscan(chrome::catalog_overscan_rows())
        // Each scroll rebuilds the list at its new offset (rows realised
        // past the last build would be blank) and fetches icons in view.
        .on_scroll(|viewport| Message::SearchScrolled(viewport.absolute_offset().y))
        .id(iced::widget::Id::new(crate::SEARCH_LIST_ID))
        .height(Length::Fill)
        .into();
    let list = enzui_widgets::scroll_view::publish_geometry_of(list, geometry);

    column![
        head_region,
        container(list)
            .padding(Padding::new(0.0).left(SpaceToken::Lg.px()))
            .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Categories
// ---------------------------------------------------------------------------

// Number of popular apps previewed in each category on the overview.
// Icon hydration consumes this same cap so every preview row can have its
// Flathub artwork without downloading the whole catalogue.
pub(crate) const CATEGORY_PREVIEW_CAP: usize = 4;

/// The Categories overview groups preview rows by category; selecting one
/// category replaces the overview with its app list.
pub(crate) fn categories_view<'a>(
    store: &'a Store,
    rows: &'a [AppEntry],
    width: f32,
) -> Element<'a, Message> {
    if let Some(key) = &store.browse_category {
        let density = RowDensity::for_width(width - 32.0);
        // Leaving a category is the titlebar Back control
        // (`ui::shell::shows_back`), not a second in-content grammar.
        let head = row![
            chrome::section_label(model::browse_category_label(key)),
            Space::new().width(Length::Fill),
            chrome::muted_caption(model::app_count_label(rows.len())),
        ]
        .spacing(SpaceToken::Space5.px())
        .align_y(Alignment::Center)
        .width(Length::Fill);

        // Preserve the category header ancestor chain across empty and populated results.
        let head_region = enz_scroll_column(vec![chrome::readable_measure(column![
            head,
            Space::new().height(Length::Fixed(SpaceToken::Md.px())),
        ])])
        .insets(
            SpaceToken::Lg.px(),
            SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER,
            SpaceToken::Lg.px(),
            0.0,
        )
        .spacing(SpaceToken::None.px())
        .width(Length::Fill)
        .height(Length::Shrink);

        if rows.is_empty() {
            let body = enz_scroll_column(vec![
                container(
                    enz_status_page(crate::i18n::label("No apps in this category yet."))
                        .icon("package")
                        .description(crate::i18n::label(
                            "There are no matching apps in the current catalogue.",
                        )),
                )
                .width(Length::Fill)
                .into(),
            ])
            .insets(
                SpaceToken::Lg.px(),
                SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER,
                0.0,
                SpaceToken::Lg.px(),
            )
            .width(Length::Fill)
            .height(Length::Fill);
            return column![head_region, body]
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }

        let provider = store.row_provider(rows);
        let operations = &store.operations;
        let repositories = store.repository_answer();
        let geometry = store.category_scroll.clone();
        let live = geometry.get();

        let list: Element<Message> =
            enz_virtual_list(provider, chrome::row_pitch(), move |_i, entry| {
                container(result_row_with_operation(
                    entry,
                    density,
                    crate::operation_for_in(operations, &entry.id),
                    None,
                    repositories,
                ))
                .padding(
                    Padding::new(0.0)
                        // The page's trailing inset lives in each row, so the
                        // list (and its scrollbar) reaches the window edge.
                        .right(SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER)
                        .bottom(chrome::ROW_GAP.px()),
                )
                .width(Length::Fill)
                .into()
            })
            .viewport_height(live.map_or(crate::ui::library::FIRST_FRAME_VIEWPORT, |g| {
                g.bounds.height
            }))
            .scroll_offset(live.map_or(0.0, |g| g.translation.y))
            .overscan(chrome::catalog_overscan_rows())
            // Each scroll rebuilds the list at its new offset (rows realised
            // past the last build would be blank) and fetches icons in view.
            .on_scroll(|viewport| Message::CategoryScrolled(viewport.absolute_offset().y))
            .id(iced::widget::Id::new(crate::CATEGORY_LIST_ID))
            .height(Length::Fill)
            .into();
        let list = enzui_widgets::scroll_view::publish_geometry_of(list, geometry);

        return column![
            head_region,
            container(list)
                .padding(Padding::new(0.0).left(SpaceToken::Lg.px()))
                .height(Length::Fill),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }

    let counts = store.category_counts();
    // Empty categories must not consume browse slots with unreachable destinations.
    let populated: Vec<(&'static str, usize)> = BROWSE_CATEGORIES
        .iter()
        .filter_map(|key| {
            let count = counts.get(*key).copied().unwrap_or(0);
            (count > 0).then_some((*key, count))
        })
        .collect();

    if populated.is_empty() {
        // Every category empty means the catalogue has not arrived, not that
        // the taxonomy is empty. Distinct copy from the per-category
        // "No apps in this category yet." above. The `Browse by category`
        // heading stays: the page still is that page, and dropping its title
        // with its content is how an empty state loses its identity.
        return enz_scroll_column(vec![
            chrome::section_label(crate::i18n::label("Browse by category")),
            Space::new()
                .height(Length::Fixed(SpaceToken::Md.px()))
                .into(),
            container(
                enz_status_page(crate::i18n::label("No categories yet."))
                    .icon("layout-grid")
                    .description(crate::i18n::label(
                        "Categories appear once the Flathub catalogue has loaded.",
                    )),
            )
            .width(Length::Fill)
            .into(),
        ])
        .inset(chrome::page_inset())
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }

    // Rows sit in grid cells, so they take the cell's density.
    let density = crate::ui::discover::section_row_density(width - 32.0);
    let candidates: Vec<&AppEntry> = store.catalog_candidates();

    let mut body: Vec<Element<Message>> = Vec::new();
    // Track which app ids have already appeared to avoid duplicating an app
    // across category sections.
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (key, count) in &populated {
        // Get apps in this category, ranked by downloads (popularity from
        // catalogue metadata, not fixture order).
        let cat_rows = model::category_rows_from(candidates.iter().copied(), key);
        // Candidates have unique IDs and one category, so a populated category retains rows after cross-section deduplication.
        let unique_rows: Vec<AppEntry> = cat_rows
            .iter()
            .filter(|e| !seen_ids.contains(&e.id))
            .take(CATEGORY_PREVIEW_CAP)
            .cloned()
            .collect();
        // Track shown ids.
        for e in &unique_rows {
            seen_ids.insert(e.id.clone());
        }

        // The complete heading band is one route affordance. Keeping the
        // count inside the same title-height line box fixes its optical
        // baseline, while the full-width clickable supplies one coherent
        // hover target instead of a title patch plus a separate action.
        let category_name = model::browse_category_label(key);
        let heading_row = enz_clickable(
            row![
                enzui_widgets::icon::enz_icon(model::browse_category_icon(key))
                    .size(SizingTokens::default().icon_lg)
                    .tint_resolved(|t| crate::theme::palette(t).accent),
                enz_text(category_name.clone())
                    .size(TextSize::Title2)
                    .weight(iced::font::Weight::Semibold)
                    .color_resolved(|t| crate::theme::palette(t).text)
                    .heading(enzui_core::semantics::HeadingLevel::Section),
                enz_text(model::app_count_label(*count))
                    .size(TextSize::Caption)
                    .line_height(iced::widget::text::LineHeight::Absolute(
                        TextSize::Title2.line_height_px().into(),
                    ))
                    .color_resolved(category_section_count_ink),
                Space::new().width(Length::Fill),
                enzui_widgets::icon::enz_icon("chevron-right")
                    .size(SizingTokens::default().icon_sm)
                    .tint_resolved(|t| crate::theme::palette(t).text_muted),
            ]
            .spacing(SpaceToken::Sm.px())
            .align_y(Alignment::Center)
            .width(Length::Fill),
            Message::CategorySelected(key.to_string()),
        )
        .aria_label(crate::i18n::format(
            crate::i18n::localizer(),
            "Open {category_name} category",
            &[("category_name", &(category_name))],
        ))
        .align_x(Alignment::Start)
        .align_y(Alignment::Center)
        .padding(Padding::from([SpaceToken::Xs.px(), SpaceToken::Sm.px()]))
        .radius(SpaceToken::Sm.px())
        .width(Length::Fill);

        // Share Discover's grid so capped category rows align with their headings.
        let cells: Vec<enzui_widgets::responsive_grid::CellFn<'a, Message>> = unique_rows
            .into_iter()
            .map(|entry| {
                let operations = &store.operations;
                let repositories = store.repository_answer();
                let f: enzui_widgets::responsive_grid::CellFn<'a, Message> = Box::new(move || {
                    result_row_with_operation(
                        &entry,
                        density,
                        crate::operation_for_in(operations, &entry.id),
                        None,
                        repositories,
                    )
                });
                f
            })
            .collect();
        let grid: Element<'a, Message> = enzui_widgets::responsive_grid::enz_responsive_grid(cells)
            .ramp(crate::ui::discover::section_ramp(width - 32.0))
            .gap_x(SpaceToken::Lg)
            .gap_y(SpaceToken::Xs)
            .into();
        let section = column![heading_row, grid]
            .spacing(SpaceToken::Sm.px())
            .width(Length::Fill);

        if !body.is_empty() {
            body.push(
                Space::new()
                    .height(Length::Fixed(SpaceToken::Xl.px()))
                    .into(),
            );
        }
        body.push(section.into());
    }

    enz_scroll_column(body)
        .inset(chrome::page_inset())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
