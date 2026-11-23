//! Search as a popup under the titlebar's search icon.
//!
//! The field searches as you type and lists the best matches; a match opens
//! its page, and "Show all results" (or Enter) opens the Search page with
//! its filters. With nothing typed, recent searches are offered.

use enzui_theme::spacing::SpaceToken;
use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_widgets::pressable::{PressableStyle, enz_pressable};
use enzui_widgets::search_field::enz_search_field;
use iced::widget::{column, container, row};
use iced::{Alignment, Element, Length};

use crate::ui::chrome;
use crate::{Message, Store};

/// The popup's field: the Ctrl+F target.
pub(crate) const SEARCH_POPUP_FIELD_ID: &str = "store-search";
/// The popup's focus scope.
pub(crate) const SEARCH_POPUP_SCOPE: &str = "store-search-popup";
/// How many matches the popup lists.
pub(crate) const SEARCH_POPUP_RESULTS: usize = 6;
pub(crate) const SEARCH_POPUP_WIDTH: f32 = 420.0;
const RESULT_ICON_PX: f32 = 32.0;

/// The popup's content.
pub(crate) fn content(store: &Store) -> Element<'_, Message> {
    let field = enz_search_field(&store.search_query, Message::SearchPopupInput)
        .placeholder(crate::i18n::text(crate::ui::topbar::SEARCH_PLACEHOLDER))
        .on_clear(Message::SearchPopupInput(String::new()))
        .on_submit(Message::SearchPopupShowAll)
        .width(Length::Fill)
        .focus_on_mount(true)
        .id(iced::advanced::widget::Id::new(SEARCH_POPUP_FIELD_ID));

    let mut body = column![field]
        .spacing(SpaceToken::Sm.px())
        .width(Length::Fill);

    if store.search_query.trim().is_empty() {
        let recent: Vec<&String> = store.recent_searches.iter().take(5).collect();
        if recent.is_empty() {
            body = body.push(hint(crate::i18n::label(
                "Search apps by name, description or keyword.",
            )));
        } else {
            body = body.push(chrome::section_label(crate::i18n::label("Recent Searches")));
            for query in recent {
                body = body.push(recent_row(query));
            }
        }
        return body.into();
    }

    if store.search_popup_results.is_empty() {
        body = body.push(hint(crate::i18n::label("No apps match.")));
        return body.into();
    }
    let mut list = column![].spacing(2.0).width(Length::Fill);
    for entry in &store.search_popup_results {
        list = list.push(result_row(entry));
    }
    body = body.push(list);
    if store.search_popup_total > store.search_popup_results.len() {
        body = body.push(
            container(
                enzui_widgets::button::enz_button(
                    crate::i18n::format(
                        crate::i18n::localizer(),
                        "Show all {count} results",
                        &[("count", &store.search_popup_total)],
                    ),
                    Message::SearchPopupShowAll,
                )
                .kind(enzui_widgets::ButtonKind::Quiet)
                .size(enzui_widgets::button::ButtonSize::Sm)
                .icon("arrow-right")
                .icon_position(enzui_widgets::button::ButtonIconPosition::Trailing),
            )
            .width(Length::Fill)
            .align_x(Alignment::End),
        );
    }
    body.into()
}

fn hint<'a>(text: &'static str) -> Element<'a, Message> {
    container(
        enz_text(text.to_owned())
            .size(TextSize::Footnote)
            .tone(TextTone::Muted),
    )
    .padding(
        iced::Padding::new(0.0)
            .top(SpaceToken::Xs.px())
            .left(SpaceToken::Xs.px()),
    )
    .into()
}

/// One match: icon, name and summary; it opens the app's page.
fn result_row<'a>(entry: &crate::services::AppEntry) -> Element<'a, Message> {
    let text = column![
        enz_text(entry.name.clone())
            .size(TextSize::Body)
            .weight(iced::font::Weight::Semibold)
            .single_line_elided()
            .full_width(),
        enz_text(entry.summary.clone())
            .size(TextSize::Footnote)
            .tone(TextTone::Muted)
            .single_line_elided()
            .full_width(),
    ]
    .spacing(0.0)
    .width(Length::Fill);
    let line = row![
        chrome::app_icon(entry.icon_image.as_ref(), &entry.name, RESULT_ICON_PX),
        text,
    ]
    .spacing(SpaceToken::Md.px())
    .align_y(Alignment::Center);
    enz_pressable(line.into(), Message::open_app_detail(entry))
        .style(PressableStyle::Row)
        .padding(SpaceToken::Sm.px())
        .width(Length::Fill)
        .aria_label(entry.name.clone())
        .into_element()
}

/// One recent search: puts the query back in the field.
fn recent_row<'a>(query: &str) -> Element<'a, Message> {
    let line = row![
        enzui_widgets::icon::enz_icon("history")
            .size(16.0)
            .into_element(),
        enz_text(query.to_owned())
            .size(TextSize::Body)
            .single_line_elided()
            .full_width(),
    ]
    .spacing(SpaceToken::Md.px())
    .align_y(Alignment::Center);
    enz_pressable(line.into(), Message::SearchPopupInput(query.to_owned()))
        .style(PressableStyle::Row)
        .padding(iced::Padding::new(SpaceToken::Sm.px()).left(SpaceToken::Md.px()))
        .width(Length::Fill)
        .aria_label(query.to_owned())
        .into_element()
}
