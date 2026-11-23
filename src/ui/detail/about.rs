//! "About this app": an information grid and the publisher's links.

use super::*;
use enzui_widgets::button::ButtonIconPosition;
use enzui_widgets::responsive_grid::{CellFn, ColumnRamp, enz_responsive_grid};
use iced::widget::column;

/// The information grid's column ramp, read off the grid's own width.
///
/// The page sits on the 960 px readable measure, so a desktop window lands in
/// the `Medium` class (three columns), a half-screen window in `Regular` (two)
/// and a phone-width one in `Compact` (still two: the values are short, and
/// an id with no break opportunity wraps by glyph). A cell is a short label over a
/// short value, so three columns of ~300 px keep every value on one line while
/// the block stays a third of the height of a single label/value list.
const INFO_RAMP: ColumnRamp = ColumnRamp {
    wide: 3,
    medium: 3,
    narrow: 2,
    compact: 2,
};

/// What one information cell shows under its label.
#[derive(Clone)]
enum FactValue {
    Text(String),
    /// The support status, a toned chip rather than plain text.
    Support {
        end_of_life: bool,
    },
    /// Still loading: a placeholder in the value's place, so the grid keeps
    /// its final order and size when the value arrives.
    Pending,
}

/// Whether the detail metadata (license, dates, links) is still on its way.
fn metadata_pending(store: &Store) -> bool {
    matches!(
        store
            .detail_context()
            .map(|context| &context.metadata_state),
        None | Some(crate::services::DetailRouteState::Loading)
    )
}

/// Add a text fact unless its value is empty.
fn push_text(facts: &mut Vec<(&'static str, FactValue)>, label: &'static str, value: String) {
    if !value.trim().is_empty() {
        facts.push((label, FactValue::Text(value)));
    }
}

/// The facts the grid shows, in reading order; empty values are dropped.
fn about_facts(
    store: &Store,
    entry: Option<&crate::services::AppEntry>,
    app_id: &str,
) -> Vec<(&'static str, FactValue)> {
    let mut facts: Vec<(&'static str, FactValue)> = Vec::new();

    if let Some(entry) = entry {
        push_text(
            &mut facts,
            crate::i18n::label("Version"),
            entry.version.clone(),
        );
        if !entry.category.trim().is_empty() {
            push_text(
                &mut facts,
                crate::i18n::label("Category"),
                crate::model::browse_category_label(&entry.category),
            );
        }
        let raw_license = if entry.license.trim().is_empty() {
            store
                .detail_context()
                .map(|context| context.license.as_str())
                .unwrap_or_default()
        } else {
            entry.license.as_str()
        };
        if !raw_license.trim().is_empty() {
            push_text(
                &mut facts,
                crate::i18n::label("License"),
                crate::services::pretty_license(raw_license),
            );
        } else if metadata_pending(store) {
            facts.push((crate::i18n::label("License"), FactValue::Pending));
        }
    }
    if metadata_pending(store) {
        facts.push((crate::i18n::label("Last updated"), FactValue::Pending));
    } else if let Some(context) = store.detail_context()
        && !matches!(
            context.metadata_state,
            crate::services::DetailRouteState::Failed
        )
    {
        push_text(
            &mut facts,
            crate::i18n::label("Last updated"),
            context.last_updated.clone(),
        );
        push_text(
            &mut facts,
            crate::i18n::label("Download size"),
            crate::services::format_byte_size(context.download_size),
        );
    }
    if let Some(entry) = entry {
        push_text(
            &mut facts,
            crate::i18n::label("Installed size"),
            entry.size.clone(),
        );
    }
    if !store.detail_from_enzimos(app_id) {
        match store.detail_context() {
            Some(context) => match context.stats_state {
                crate::services::DetailRouteState::Known => push_text(
                    &mut facts,
                    crate::i18n::label("Monthly installs"),
                    crate::model::format_installs(context.installs_last_month),
                ),
                crate::services::DetailRouteState::KnownEmpty => push_text(
                    &mut facts,
                    crate::i18n::label("Monthly installs"),
                    crate::i18n::label("No installs reported").to_owned(),
                ),
                crate::services::DetailRouteState::Failed => push_text(
                    &mut facts,
                    crate::i18n::label("Install statistics"),
                    crate::i18n::label("Unavailable").to_owned(),
                ),
                crate::services::DetailRouteState::Loading => {
                    facts.push((crate::i18n::label("Monthly installs"), FactValue::Pending))
                }
                crate::services::DetailRouteState::CachedStale { age_secs } => {
                    push_text(
                        &mut facts,
                        crate::i18n::label("Monthly installs"),
                        crate::model::format_installs(context.installs_last_month),
                    );
                    push_text(
                        &mut facts,
                        crate::i18n::label("Install statistics"),
                        crate::i18n::format(
                            crate::i18n::localizer(),
                            "Cached from {} ago; refreshing",
                            &[(
                                "arg0",
                                &(crate::model::format_age(age_secs.saturating_mul(1000))),
                            )],
                        ),
                    );
                }
            },
            None => facts.push((crate::i18n::label("Monthly installs"), FactValue::Pending)),
        }
    }
    if let Some(entry) = entry {
        push_text(
            &mut facts,
            crate::i18n::label("Branch"),
            entry.branch.clone(),
        );
    }
    push_text(&mut facts, crate::i18n::label("App ID"), app_id.to_owned());
    if let Some(entry) = entry {
        facts.push((
            crate::i18n::label("Support"),
            FactValue::Support {
                end_of_life: entry.is_eol,
            },
        ));
    }
    facts
}

/// One information cell: a muted caption over its value.
fn info_cell<'a>(label: &'static str, value: &FactValue) -> Element<'a, Message> {
    let value: Element<'a, Message> = match value {
        FactValue::Text(text) => enz_text(text.clone())
            .size(TextSize::Body)
            .tone(TextTone::Default)
            // App ids and versions have no spaces to break at.
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .width(Length::Fill)
            .into(),
        // `triangle-alert`, not `alert-triangle`: the latter is the pre-rename
        // upstream name and is absent from the registry, so the one chip on
        // this page that warns about an unsupported app would draw the
        // fallback glyph.
        FactValue::Support { end_of_life: true } => chrome::icon_chip(
            "triangle-alert",
            crate::i18n::label("End of life"),
            chrome::ChipTone::Warning,
        ),
        // A Body line's height, a typical value's width.
        FactValue::Pending => {
            chrome::skeleton_block(Length::Fixed(96.0), 18.0, crate::i18n::label("Loading…"))
        }
        FactValue::Support { end_of_life: false } => chrome::icon_chip(
            "shield-check",
            crate::i18n::label("Supported"),
            chrome::ChipTone::Success,
        ),
    };
    column![
        enz_text(label.to_owned())
            .size(TextSize::Footnote)
            .tone(TextTone::Muted)
            .single_line_elided()
            .full_width(),
        value,
    ]
    .spacing(SpaceToken::Xxs.px())
    .width(Length::Fill)
    .into()
}

/// The information grid, followed by the publisher's links when it has any.
pub(crate) fn about_section<'a>(
    store: &'a Store,
    entry: Option<&crate::services::AppEntry>,
    app_id: &str,
) -> Element<'a, Message> {
    let cells: Vec<CellFn<'a, Message>> = about_facts(store, entry, app_id)
        .into_iter()
        .map(|(label, value)| {
            let f: CellFn<'a, Message> = Box::new(move || info_cell(label, &value));
            f
        })
        .collect();
    let grid: Element<'a, Message> = enz_responsive_grid(cells)
        .ramp(INFO_RAMP)
        .gap_x(SpaceToken::Xl)
        .gap_y(SpaceToken::Lg)
        .into();

    let mut block = column![grid]
        .spacing(SpaceToken::Lg.px())
        .width(Length::Fill);
    if let Some(links) = about_links(store) {
        block = block.push(links);
    } else if metadata_pending(store) {
        // Most apps link a homepage and more: hold one row of link buttons.
        let mut row = iced::widget::Row::new().spacing(SpaceToken::Sm.px());
        for width in [104.0, 112.0, 88.0] {
            row = row.push(chrome::skeleton_block(
                Length::Fixed(width),
                26.0,
                crate::i18n::label("Loading…"),
            ));
        }
        block = block.push(row);
    }
    block.into()
}

/// The publisher's links as one wrapping row of link buttons.
fn about_links<'a>(store: &'a Store) -> Option<Element<'a, Message>> {
    let context = store.detail_context()?;
    if context.urls.is_empty() {
        return None;
    }
    let mut items: Vec<Element<'a, Message>> = Vec::new();
    for (label, icon, id, url) in [
        (
            crate::i18n::label("Homepage"),
            "globe",
            "store-publisher-homepage",
            &context.urls.homepage,
        ),
        (
            crate::i18n::label("Bug tracker"),
            "bug",
            "store-publisher-bugs",
            &context.urls.bugtracker,
        ),
        (
            crate::i18n::label("Donate"),
            "heart",
            "store-publisher-donate",
            &context.urls.donation,
        ),
        (
            crate::i18n::label("Help"),
            "life-buoy",
            "store-publisher-help",
            &context.urls.help,
        ),
    ] {
        if url.trim().is_empty() {
            continue;
        }
        items.push(
            enz_button(label, Message::OpenUrl(url.clone()))
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .icon(icon)
                .icon_position(ButtonIconPosition::Leading)
                // It navigates to a web page, so it is announced as a link
                // and says where it goes.
                .role(enzui_core::semantics::Role::Link)
                .aria_description(url.clone())
                .tooltip(url.clone())
                .id(iced::advanced::widget::Id::new(id))
                .into_element(),
        );
    }
    if items.is_empty() {
        return None;
    }
    let mut block = column![
        iced::widget::Row::with_children(items)
            .spacing(SpaceToken::Sm.px())
            .align_y(Alignment::Center)
            .wrap()
            .vertical_spacing(SpaceToken::Sm.px()),
    ]
    .spacing(SpaceToken::Sm.px())
    .width(Length::Fill);
    if let Some(error) = &store.last_url_error {
        block = block.push(
            enz_text(crate::i18n::text(error))
                .size(TextSize::Footnote)
                .color_resolved(|theme| crate::theme::palette(theme).danger),
        );
    }
    Some(block.into())
}
