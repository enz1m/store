//! Store views and shared window chrome.

pub(crate) mod chrome;
pub(crate) mod detail;
pub(crate) mod discover;
pub(crate) mod library;
pub(crate) mod remotes;
pub(crate) mod repositories;
pub(crate) mod search;
pub(crate) mod search_popup;
pub(crate) mod shell;
pub(crate) mod topbar;

use crate::{Message, Route, Store};
use iced::widget::container;
use iced::{Element, Length};

// Preserve page state by keeping closed and open modal hosts at the same tree depth.
pub(crate) fn closed_dialog_host<'a>(page: Element<'a, Message>) -> Element<'a, Message> {
    enzui_widgets::modal_host::enz_modal_host(page, None, |_| Message::DialogHostIdle)
        .into_element()
}

// Borrow precomputed projections; layout can invoke this closure repeatedly.
fn shell_content<'a>(store: &'a Store, width: f32) -> Element<'a, Message> {
    let page: Element<'a, Message> = match store.route {
        Route::Library => library::view(store, store.rows(), width),
        Route::Search => search::view(store, store.rows(), width),
        Route::Categories => search::categories_view(store, store.rows(), width),
        Route::Detail => match store.detail_app_id_ref() {
            Some(id) => detail::view(store, id, width),
            None => container(
                enzui_widgets::status_page::enz_status_page(crate::i18n::label("No app selected"))
                    .icon("info")
                    .primary_action(enzui_core::action::Action::new(
                        crate::i18n::label("Open Library"),
                        Message::Navigate(Route::Library),
                    )),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        },
        Route::Discover => discover::view(store, width),
    };
    enzui_core::collect::app_subtree_boundary(page).into()
}
