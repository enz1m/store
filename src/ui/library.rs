//! Library — installed apps, live downloads, recently uninstalled, remotes.
//!
//! The primary list leads with running work ("In progress", one progress bar
//! per row), then apps with a pending update, then apps that are up to date.
//! Finished tasks and removal history share a secondary panel that stays
//! closed until the header's Activity button opens it.
//!
//! Virtualized rows bound image handles to the visible portion of the catalog.

use crate::model::LibrarySort;
use crate::services::AppEntry;
use crate::ui::chrome::{self, ChipTone};
use crate::{Message, Store};
use enzui_widgets::enz_text::{TextSize, enz_text};

use enzui_theme::spacing::SpaceToken;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_widgets::cached_subtree::enz_cached_subtree;
use enzui_widgets::scroll_column::enz_scroll_column;
use enzui_widgets::search_field::enz_search_field;
use enzui_widgets::status_page::enz_status_page;
use enzui_widgets::surface::enz_surface;
use enzui_widgets::toolbar_icon::enz_toolbar_icon;
use enzui_widgets::virtual_list::enz_virtual_list;
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Padding};
use std::rc::Rc;

/// Optional row content follows the available width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RowDensity {
    /// Phone. Icon-only actions, and the subtitle is a single line: version
    /// and size only.
    Narrow,
    /// Small laptop / tablet. Icon-only actions; the update chip is back.
    Compact,
    /// Everything: labelled Update button and the origin chip.
    Wide,
}

pub(crate) const ROW_WIDE_MIN_WIDTH: f32 = 620.0;

pub(crate) const HEADER_SUMMARY_MIN_WIDTH: f32 = 430.0;

const LIBRARY_SECONDARY_MIN_WIDTH: f32 = 900.0;
const LIBRARY_SECONDARY_WIDTH: f32 = 380.0;

pub(crate) const ROW_COMPACT_MIN_WIDTH: f32 = 420.0;

impl RowDensity {
    pub(crate) fn for_width(content_width: f32) -> Self {
        if content_width >= ROW_WIDE_MIN_WIDTH {
            RowDensity::Wide
        } else if content_width >= ROW_COMPACT_MIN_WIDTH {
            RowDensity::Compact
        } else {
            RowDensity::Narrow
        }
    }
    /// The origin chip ("flathub") is the first thing to go.
    pub(crate) fn shows_origin(self) -> bool {
        matches!(self, RowDensity::Wide)
    }
    /// A labelled "Update" button, versus a bare icon button.
    pub(crate) fn shows_action_labels(self) -> bool {
        matches!(self, RowDensity::Wide)
    }
    /// The "Update available" subtitle chip. Redundant with the update button
    /// itself, so it is the second thing to go.
    pub(crate) fn shows_update_chip(self) -> bool {
        !matches!(self, RowDensity::Narrow)
    }
    /// The install-count chip in search results.
    pub(crate) fn shows_installs(self) -> bool {
        matches!(self, RowDensity::Wide)
    }
}

// Overestimate the initial viewport so virtualization cannot omit rows before geometry arrives.
pub(crate) const FIRST_FRAME_VIEWPORT: f32 = 4320.0;

/// Build the Library view for a resolved content width.
pub(crate) fn view<'a>(store: &'a Store, rows: &'a [AppEntry], width: f32) -> Element<'a, Message> {
    let density = RowDensity::for_width(width - 32.0);
    let mut body: Vec<Element<Message>> = Vec::new();
    let mut secondary: Vec<Element<Message>> = Vec::new();

    // Running work leads the main list; settled history sits behind the
    // Activity button so it never crowds the installed apps by default.
    // An installed app's running update or removal shows in its own row
    // below; this section is for what has no row yet (a new install) and for
    // failures, which stay until dismissed.
    let in_progress: Vec<&crate::model::OperationState> = store
        .operations
        .iter()
        .filter(|o| {
            o.error.is_some()
                || (!o.finished && !store.installed.iter().any(|entry| entry.id == o.app_id))
        })
        .collect();
    let history: Vec<&crate::model::OperationState> = store
        .operations
        .iter()
        .filter(|o| o.finished && o.error.is_none())
        .collect();
    let has_history = !history.is_empty() || !store.recently_uninstalled.is_empty();

    if store.library_activity_open && has_history {
        let mut header = row![chrome::section_label(crate::i18n::label("Activity"))]
            .spacing(SpaceToken::Space5.px())
            .align_y(Alignment::Center)
            .width(Length::Fill);
        header = header.push(Space::new().width(Length::Fill));
        if !history.is_empty() {
            header = header.push(
                enz_button(crate::i18n::label("Clear"), Message::ClearFinishedTasks)
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .aria_label(crate::i18n::label("Clear finished tasks")),
            );
        }
        header = header.push(
            enz_toolbar_icon("panel-right-close", Message::LibraryActivityToggled)
                .aria_label(crate::i18n::label(ACTIVITY_CLOSE_LABEL))
                .tooltip(crate::i18n::label("Close activity"))
                .id(iced::advanced::widget::Id::new(
                    "store-library-activity-close",
                )),
        );
        secondary.push(header.into());
        if history.is_empty() {
            secondary.push(chrome::muted_caption(crate::i18n::label(
                "No finished tasks",
            )));
        }
        for op in &history {
            secondary.push(operation_row(op, store.app_icon_image(&op.app_id)));
        }
    }

    // ---- Installed ------------------------------------------------------
    let sort_options: Vec<(String, Message)> =
        [LibrarySort::Name, LibrarySort::Size, LibrarySort::Updates]
            .into_iter()
            .map(|sort| (sort.label().to_owned(), Message::LibrarySortChanged(sort)))
            .collect();
    let selected_sort = store.library_sort.label().to_owned();

    // The header is over-subscribed at narrow widths: at 375px the content box
    // is 331px and "INSTALLED" + the summary + the Sort-by combo + the
    // repositories icon do not fit. So
    // the summary caption — the only part carrying no affordance — is dropped
    // below `HEADER_SUMMARY_MIN_WIDTH`, and it moves under the section label
    // rather than vanishing.
    let summary = if rows.is_empty() {
        String::new()
    } else {
        format!(
            "{} · {}",
            crate::model::app_count_label(rows.len()),
            crate::model::library_total_size(&store.installed)
        )
    };
    let header_width = width - 32.0 - chrome::SCROLLBAR_GUTTER;
    let inline_summary = header_width >= HEADER_SUMMARY_MIN_WIDTH;

    let mut installed_header = row![
        enz_text(crate::i18n::label("Installed").to_owned())
            .size(TextSize::Title3)
            .weight(iced::font::Weight::Semibold)
            .heading(enzui_core::semantics::HeadingLevel::Section)
    ]
    .spacing(SpaceToken::Lg.px())
    .align_y(Alignment::Center)
    .width(Length::Fill);
    if inline_summary && !rows.is_empty() {
        installed_header = installed_header.push(library_stats(store, rows.len()));
    }
    let mut trailing = row![Space::new().width(Length::Fill)]
        .spacing(SpaceToken::Sm.px())
        .align_y(Alignment::Center);
    if has_history {
        let open = store.library_activity_open;
        let toggle: Element<Message> = enz_button(
            crate::i18n::label("Activity"),
            Message::LibraryActivityToggled,
        )
        .kind(ButtonKind::Quiet)
        .size(ButtonSize::Sm)
        .aria_label(if open {
            crate::i18n::label(ACTIVITY_CLOSE_LABEL)
        } else {
            crate::i18n::label(ACTIVITY_OPEN_LABEL)
        })
        .id(iced::advanced::widget::Id::new(ACTIVITY_TOGGLE_ID))
        .into();
        trailing = trailing.push(toggle);
    }
    // The "Sort by" caption is the first thing a narrow header drops: it
    // elided to "…" beside the combo at 480 px, and the combo's own
    // accessible name and its value already say what it sorts by.
    if header_width >= ROW_WIDE_MIN_WIDTH {
        trailing = trailing.push(chrome::muted_caption(crate::i18n::label("Sort by")));
    }
    let installed_header = installed_header.push(trailing.extend([
        // Sorting is a field value, not three adjacent actions. The
        // compact combo gives the control one continuous outlined frame
        // and a conventional menu instead of a visually fragmented strip.
        chrome::compact_select(
            crate::i18n::label("Sort by"),
            &selected_sort,
            store.library_sort_menu_open,
            Message::LibrarySortMenu(true),
            Message::LibrarySortMenu(false),
            sort_options,
            "store-library-sort",
        ),
    ]));

    body.push(installed_header.into());

    if !store.installed.is_empty() {
        body.push(
            container(
                enz_search_field(&store.library_query, Message::LibrarySearchInput)
                    .placeholder(crate::i18n::text("Search installed apps"))
                    .on_clear(Message::LibrarySearchCleared)
                    .width(Length::Fill)
                    .id(iced::advanced::widget::Id::new("store-library-search")),
            )
            .padding(
                Padding::new(0.0)
                    .top(SpaceToken::Sm.px())
                    .bottom(SpaceToken::Xs.px())
                    .right(chrome::SCROLLBAR_GUTTER),
            )
            .width(Length::Fill)
            .into(),
        );
    }

    if !inline_summary && !summary.is_empty() {
        body.push(
            container(library_stats(store, rows.len()))
                .padding(
                    Padding::new(0.0)
                        .left(SpaceToken::Xs.px())
                        .bottom(SpaceToken::Xs.px()),
                )
                .into(),
        );
    }
    // ---- In progress -------------------------------------------------------
    // Running installs, updates and removals lead the list, one row and one
    // progress bar each. A failure stays here, as its assertive alert, until
    // the user dismisses it.
    if !in_progress.is_empty() {
        let running = in_progress.iter().filter(|o| !o.finished).count();
        let failed = in_progress.len() - running;
        body.push(
            Space::new()
                .height(Length::Fixed(SpaceToken::Sm.px()))
                .into(),
        );
        let mut header = row![chrome::section_label(crate::i18n::label(IN_PROGRESS_LABEL))]
            .spacing(SpaceToken::Sm.px())
            .align_y(Alignment::Center)
            .width(Length::Fill);
        // The count on the far side, as the other sections' headings have it.
        header = header.push(Space::new().width(Length::Fill));
        if running > 0 {
            header = header.push(chrome::muted_caption(crate::model::task_count_label(
                running,
            )));
        }
        if failed > 0 {
            header = header.push(
                enz_button(
                    crate::i18n::label("Dismiss failed"),
                    Message::DismissFailedTasks,
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm),
            );
        }
        body.push(
            container(header)
                .padding(Padding::new(0.0).bottom(SpaceToken::Xs.px()))
                .width(Length::Fill)
                .into(),
        );
        for op in &in_progress {
            body.push(
                container(operation_row(op, store.app_icon_image(&op.app_id)))
                    .padding(Padding::new(0.0).bottom(chrome::ROW_GAP.px()))
                    .width(Length::Fill)
                    .into(),
            );
        }
    }

    body.push(
        Space::new()
            .height(Length::Fixed(SpaceToken::Xs.px()))
            .into(),
    );

    if rows.is_empty() {
        let filtering = !store.library_query.trim().is_empty() && !store.installed.is_empty();
        let page: Element<Message> = if let Some(err) = read_failure(store) {
            // The read failed, so the installed set is unknown rather than
            // empty. Keep both recovery actions beside an assertive alert.
            column![
                chrome::failure_banner(
                    installed_error_title(),
                    Some(installed_error_description(&err)),
                )
                .into_element(),
                row![
                    enz_button(
                        crate::i18n::label(crate::ui::discover::RETRY_LABEL),
                        Message::RefreshRequested
                    )
                    .kind(ButtonKind::Accent)
                    .icon("refresh-cw"),
                    enz_button(
                        crate::i18n::label("Browse Discover"),
                        Message::Navigate(crate::Route::Discover)
                    )
                    .kind(ButtonKind::Ghost),
                ]
                .spacing(SpaceToken::Sm.px()),
            ]
            .spacing(SpaceToken::Md.px())
            .width(Length::Fill)
            .into()
        } else if store.loading_installed {
            // A pending read must remain distinct from an authoritatively empty installed list.
            enz_status_page(crate::i18n::label("Reading flatpak state…"))
                .icon("loader")
                .description(crate::i18n::label("Asking flatpak what is installed."))
                .into()
        } else if filtering {
            // The library is non-empty but the filter matched nothing — this is
            // a "no matches" state, NOT a "nothing installed" state. Conflating
            // them would tell a user with 90 apps that they have none.
            enz_status_page(crate::i18n::format(
                crate::i18n::localizer(),
                "No installed apps match “{}”",
                &[("arg0", &(store.library_query.trim()))],
            ))
            .icon("search-x")
            .description(crate::i18n::label(
                "Clear the filter to see every installed app.",
            ))
            .into()
        } else {
            enz_status_page(crate::i18n::label("No flatpaks installed for this user."))
                .icon("package")
                .description(
                    "Apps you install from Flathub appear here. Nothing is installed \
                     automatically — every install needs an explicit click.",
                )
                .primary_action(enzui_core::action::Action::new(
                    crate::i18n::label("Browse Discover"),
                    Message::Navigate(crate::Route::Discover),
                ))
                .into()
        };
        body.push(container(page).width(Length::Fill).into());
    }

    // ---- Recently uninstalled -------------------------------------------
    if store.library_activity_open && !store.recently_uninstalled.is_empty() {
        if !secondary.is_empty() {
            secondary.push(
                Space::new()
                    .height(Length::Fixed(SpaceToken::Lg.px()))
                    .into(),
            );
        }
        secondary.push(
            row![
                chrome::section_label(crate::i18n::label("Recently removed")),
                Space::new().width(Length::Fill),
                enz_button(
                    crate::i18n::label("Clear"),
                    Message::ClearRecentlyUninstalled
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm),
            ]
            // `Space5` matches the Activity-header pattern above so
            // the two section headers use the same 10 px default gap.
            .spacing(SpaceToken::Space5.px())
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into(),
        );
        for entry in &store.recently_uninstalled {
            let id = entry.id.clone();
            let name = entry.name.clone();
            let detail_message = Message::open_app_detail(entry);
            // Reinstall requires source-repository authority, not the scope of a removed installation.
            let trailing: Element<Message> = match store.install_authorization(&entry.id).refusal()
            {
                None => enz_button(
                    crate::i18n::label("Reinstall"),
                    Message::InstallRequested(id, name),
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .aria_label(crate::i18n::format(
                    crate::i18n::localizer(),
                    "Reinstall {}",
                    &[("arg0", &(entry.name))],
                ))
                .into(),
                Some(refusal) => refusal_chip(refusal),
            };
            secondary.push(chrome::app_row(
                entry.icon_image.as_ref(),
                &entry.name,
                chrome::muted_line(entry.id.clone()),
                trailing,
                detail_message,
                None,
            ));
        }
    }

    // The list's first heading heads the list from above it: as a list item
    // it sits at the foot of a full row pitch, a band of empty space under
    // the search field.
    let mut owned_items: Vec<LibraryItem> = library_items(rows);
    if let Some(LibraryItem::Section(lead)) = owned_items.first() {
        body.push(
            container(section_line(lead))
                .padding(
                    Padding::new(0.0)
                        .top(SpaceToken::Lg.px())
                        // Clear of the first row's hover, with the section's
                        // button as tall as it is.
                        .bottom(SpaceToken::Xs.px())
                        .right(chrome::SCROLLBAR_GUTTER),
                )
                .into(),
        );
        owned_items.remove(0);
    }

    // Child blocks own their spacers, so the enclosing column adds none.
    let header_block = column(body)
        .spacing(SpaceToken::None.px())
        .width(Length::Fill);

    // Preserve the search field ancestor chain across empty/nonempty results; virtualize only the unbounded installed list.
    let empty = rows.is_empty();

    // The header block reserves the scrollbar gutter itself or its dividers
    // overshoot the rows' hairlines by exactly the scrollbar width.
    // The head shares the rows' readable measure so both start at one edge.
    let header_region = enz_scroll_column(vec![chrome::readable_measure(header_block)])
        .insets(
            SpaceToken::Lg.px(),
            SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER,
            SpaceToken::Lg.px(),
            if empty { SpaceToken::Lg.px() } else { 0.0 },
        )
        .spacing(SpaceToken::None.px())
        .width(Length::Fill)
        // Empty: the header IS the page, so it takes the height and scrolls.
        // Non-empty: it is a fixed head over the virtualised list, so it
        // shrinks to its content — the geometry the old `container` gave it.
        .height(if empty { Length::Fill } else { Length::Shrink });

    let body: Element<Message> = if empty {
        Space::new()
            .width(Length::Fill)
            .height(Length::Shrink)
            .into()
    } else {
        // Retained rows skip work during activity animation; scroll invalidation rebuilds the visible window.
        // Apps with a pending update lead under their own heading; the rest
        // follow under "Up to date". Headings are list items of the same
        // pitch, so the window arithmetic stays uniform.
        // Immutable Rc handles let repeated cold-cache builders carry owned rows and scope-specific authorization.
        let update_authorizations: Rc<
            std::collections::HashMap<String, crate::services::Authorization>,
        > = Rc::new(
            store
                .updates_available()
                .iter()
                .map(|update| {
                    (
                        update.app_id.clone(),
                        store.update_authorization(&update.app_id),
                    )
                })
                .collect(),
        );
        // Running work on an installed app reports in that app's own row.
        let running_ops: Rc<std::collections::HashMap<String, crate::model::OperationState>> =
            Rc::new(
                store
                    .operations
                    .iter()
                    .filter(|op| !op.finished)
                    .map(|op| (op.app_id.clone(), op.clone()))
                    .collect(),
            );
        let geometry = store.library_scroll.clone();
        let live = geometry.get();
        let vh = live.map_or(FIRST_FRAME_VIEWPORT, |g| g.bounds.height);
        let so = live.map_or(0.0, |g| g.translation.y);

        let realised_at = store.library_realised_at.clone();
        let cached_body: Element<Message> = enz_cached_subtree(&store.library_cache, move || {
            // The offset these rows are realised at, for the scroll reducer to
            // compare a report against.
            realised_at.set(so);

            let provider = Rc::new(LibraryItems(owned_items.clone()));
            // A fresh handle per realisation, for the same reason
            // `owned_rows` is cloned above.
            let update_authorizations = Rc::clone(&update_authorizations);
            let running_ops = Rc::clone(&running_ops);
            let list: Element<'static, Message> =
                enz_virtual_list(provider, chrome::row_pitch(), move |_i, item| {
                    let content = match item {
                        LibraryItem::Section(section) => section_row(section),
                        LibraryItem::App(entry) => installed_row(
                            entry,
                            density,
                            update_authorizations.get(&entry.id).copied(),
                            running_ops.get(&entry.id),
                        ),
                    };
                    container(content)
                        .padding(
                            Padding::new(0.0)
                                .right(SpaceToken::Lg.px() + chrome::SCROLLBAR_GUTTER)
                                .bottom(chrome::ROW_GAP.px()),
                        )
                        .width(Length::Fill)
                        .into()
                })
                .viewport_height(vh)
                .scroll_offset(so)
                .overscan(chrome::catalog_overscan_rows())
                // The scrollable owns the live offset, but the cached
                // realisation must be invalidated when that offset changes.
                // Publishing a message here keeps both owners in lock-step;
                // otherwise the scrollable can move beyond its stale realised
                // rows and expose a viewport-sized blank page.
                .on_scroll(|viewport| Message::LibraryScrolled(viewport.absolute_offset().y))
                .id(iced::widget::Id::new(crate::LIBRARY_LIST_ID))
                .height(Length::Fill)
                .into();
            // Flush trailing edge: the framework overlay bar sits at the
            // outer column edge, and each row above absorbs the trailing
            // `Lg + SCROLLBAR_GUTTER` so content columns stay put.
            container(list)
                .padding(Padding::new(0.0).left(SpaceToken::Lg.px()))
                .height(Length::Fill)
                .into()
        })
        .into();

        // publish_geometry_of must stay OUTSIDE the cached boundary: it
        // stamps the live scroll geometry on every update/operate pass,
        // which the next cold build reads for the initial viewport window.

        enzui_widgets::scroll_view::publish_geometry_of(cached_body, geometry)
    };

    let primary: Element<Message> = column![header_region, body]
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    if secondary.is_empty() {
        return primary;
    }

    let secondary_scroll = enz_scroll_column(vec![
        column(secondary)
            .spacing(SpaceToken::Sm.px())
            .width(Length::Fill)
            .into(),
    ])
    .insets(
        SpaceToken::Lg.px(),
        SpaceToken::Lg.px(),
        SpaceToken::Lg.px(),
        SpaceToken::Lg.px(),
    )
    .width(Length::Fill)
    .height(Length::Shrink);
    // A card as tall as what it lists, the page's card surface, rather than
    // a grey column the window's full height.
    let secondary_surface = enz_surface(secondary_scroll)
        .tone(enzui_widgets::surface::SurfaceTone::SurfaceAlt)
        .border_style(enzui_widgets::surface::BorderStyle::Thin)
        .border_width(1.0)
        .radius(10.0)
        .width(Length::Fill)
        .height(Length::Shrink)
        .into_element();

    if width >= LIBRARY_SECONDARY_MIN_WIDTH {
        row![
            container(primary).width(Length::Fill),
            container(secondary_surface)
                .padding(
                    Padding::new(0.0)
                        .top(SpaceToken::Lg.px())
                        .right(SpaceToken::Lg.px())
                        .bottom(SpaceToken::Lg.px()),
                )
                .width(Length::Fixed(LIBRARY_SECONDARY_WIDTH)),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        column![
            container(primary).height(Length::FillPortion(3)),
            container(secondary_surface)
                .padding(Padding::new(SpaceToken::Lg.px()).top(SpaceToken::Sm.px()))
                .height(Length::FillPortion(2)),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

/// What the automatic updates switch does.
pub(crate) const AUTOMATIC_UPDATES_DESCRIPTION: &str = "Once a day in the background, not on metered connections. An app that is open keeps its version until you close it.";

/// Accessible name of the Library's activity-history toggle while closed.
pub(crate) const ACTIVITY_OPEN_LABEL: &str = "Show activity";
/// Accessible name of the controls that close the activity-history panel.
pub(crate) const ACTIVITY_CLOSE_LABEL: &str = "Hide activity";
/// Widget id of the activity-history toggle in the Library header.
pub(crate) const ACTIVITY_TOGGLE_ID: &str = "store-library-activity-toggle";
/// Heading of the running-work section at the top of the Library.
pub(crate) const IN_PROGRESS_LABEL: &str = "In progress";
/// Heading of the installed apps with a pending update.
pub(crate) const UPDATES_SECTION_LABEL: &str = "Available Updates";
/// Heading of the installed apps without a pending update.
pub(crate) const UP_TO_DATE_SECTION_LABEL: &str = "Up to date";

/// Which installed-list heading a section row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibrarySection {
    Updates,
    UpToDate,
}

/// A heading inside the installed list, with the count it heads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SectionHeading {
    pub section: LibrarySection,
    pub count: usize,
}

/// One item of the installed virtual list: a section heading or an app.
#[derive(Clone, Debug)]
pub(crate) enum LibraryItem {
    Section(SectionHeading),
    App(Box<AppEntry>),
}

/// Group the projected rows: pending updates first, then everything that is
/// up to date. Each group keeps the selected sort order.
pub(crate) fn library_items(rows: &[AppEntry]) -> Vec<LibraryItem> {
    let (updates, current): (Vec<&AppEntry>, Vec<&AppEntry>) =
        rows.iter().partition(|entry| entry.update_available);
    let mut items = Vec::with_capacity(rows.len() + 2);
    if !updates.is_empty() {
        items.push(LibraryItem::Section(SectionHeading {
            section: LibrarySection::Updates,
            count: updates.len(),
        }));
        items.extend(
            updates
                .into_iter()
                .cloned()
                .map(|entry| LibraryItem::App(Box::new(entry))),
        );
    }
    if !current.is_empty() {
        items.push(LibraryItem::Section(SectionHeading {
            section: LibrarySection::UpToDate,
            count: current.len(),
        }));
        items.extend(
            current
                .into_iter()
                .cloned()
                .map(|entry| LibraryItem::App(Box::new(entry))),
        );
    }
    items
}

/// The owned provider behind the installed virtual list.
struct LibraryItems(Vec<LibraryItem>);

impl enzui_core::virtual_sequence::ItemProvider<LibraryItem> for LibraryItems {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn item(&self, index: u64) -> Option<LibraryItem> {
        self.0.get(usize::try_from(index).ok()?).cloned()
    }
}

impl enzui_core::virtual_sequence::KeyedProvider<LibraryItem> for LibraryItems {
    fn key(&self, index: u64) -> Option<enzui_core::virtual_sequence::SequenceKey> {
        use enzui_core::virtual_sequence::SequenceKey;
        self.0
            .get(usize::try_from(index).ok()?)
            .map(|item| match item {
                LibraryItem::Section(heading) => SequenceKey::new(match heading.section {
                    LibrarySection::Updates => "section:updates".to_owned(),
                    LibrarySection::UpToDate => "section:up-to-date".to_owned(),
                }),
                LibraryItem::App(entry) => SequenceKey::new(entry.id.clone()),
            })
    }
}

/// A section heading in the installed list, in the "Available Updates"
/// grammar: uppercase label, muted count, and trailing bulk action. It sits
/// at the bottom of its row pitch so it reads as the head of the rows below.
fn section_row<'a>(heading: &SectionHeading) -> Element<'a, Message> {
    // The same measure the app rows use, so the heading starts at their edge.
    // The search field's trailing gutter too, so every section's count ends
    // where the field and the rows' last controls do.
    container(chrome::readable_measure(section_line(heading)))
        .padding(
            Padding::new(0.0)
                .bottom(SpaceToken::Xs.px())
                .right(chrome::SCROLLBAR_GUTTER),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced::alignment::Vertical::Bottom)
        .into()
}

/// A section's label, with its count on the far side (and the section's
/// action after it).
fn section_line<'a>(heading: &SectionHeading) -> Element<'a, Message> {
    let mut line = row![]
        .spacing(SpaceToken::Sm.px())
        .align_y(Alignment::Center)
        .width(Length::Fill);
    match heading.section {
        LibrarySection::Updates => {
            line = line.extend([
                chrome::section_label(crate::i18n::label(UPDATES_SECTION_LABEL)),
                Space::new().width(Length::Fill).into(),
                chrome::muted_caption(crate::model::update_count_label(heading.count)),
            ]);
            // One update has its own button on its row.
            if heading.count > 1 {
                line = line.push(
                    enz_button(
                        crate::ui::detail::update_all_label(),
                        Message::UpdateAllRequested,
                    )
                    // The section's own action, styled like the rows' Update
                    // buttons it does all at once.
                    .kind(ButtonKind::Accent)
                    .size(ButtonSize::Sm)
                    .aria_label(crate::ui::detail::update_all_label()),
                );
            }
        }
        LibrarySection::UpToDate => {
            line = line.extend([
                chrome::section_label(crate::i18n::label(UP_TO_DATE_SECTION_LABEL)),
                Space::new().width(Length::Fill).into(),
                chrome::muted_caption(crate::model::app_count_label(heading.count)),
            ]);
        }
    }
    line.into()
}

/// The installed apps' count and total size, as the rows show their own
/// metadata: a muted glyph beside each value.
fn library_stats<'a>(store: &Store, count: usize) -> Element<'a, Message> {
    row![
        chrome::icon_chip(
            "package",
            crate::model::app_count_label(count),
            ChipTone::Muted
        ),
        chrome::icon_chip(
            "hard-drive",
            crate::model::library_total_size(&store.installed),
            ChipTone::Muted
        ),
    ]
    .spacing(SpaceToken::Md.px())
    .align_y(Alignment::Center)
    .into()
}

#[path = "library/rows.rs"]
mod rows;

use rows::{installed_error_description, installed_error_title, operation_row, read_failure};
pub(crate) use rows::{
    installed_row, remote_is_removable, remote_scope, remote_title, repositories_error_description,
    repositories_error_title, update_check_notice,
};
pub(crate) use rows::{notice_chip, refusal_chip};
