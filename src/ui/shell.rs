//! Responsive Store shell and routed content.
//!
//! Navigation uses a separate subtitle row only when the action bar stacks.

use enzui_widgets::app_shell::{ShellLayout, ShellTokens, enz_app_shell};
use iced::Element;

use crate::ui::topbar::BarMode;
use crate::{Message, Store};

pub(crate) const TITLEBAR_HEIGHT: f32 = 38.0;

pub(crate) fn shell_tokens(subtitlebar_height: f32) -> ShellTokens {
    ShellTokens {
        subtitlebar_height,
        titlebar_height: TITLEBAR_HEIGHT,
        ..ShellTokens::default()
    }
}

pub(crate) fn shows_back(store: &Store) -> bool {
    store.route.is_detail()
        || (store.route == crate::Route::Categories && store.browse_category.is_some())
}

/// Build the responsive shell; modal overlays are hosted by the caller.
pub(crate) fn view<'a>(store: &'a Store, width: f32) -> Element<'a, Message> {
    let mode = BarMode::for_width(width);
    let sub_h = if mode.nav_is_stacked() {
        crate::ui::topbar::NAV_ROW_HEIGHT + 1.0
    } else {
        0.0
    };

    let route = store.route;
    let stacked_activity = crate::ui::topbar::NavActivity::of(store);
    let mut shell = enz_app_shell(move || super::shell_content(store, width))
        .layout(ShellLayout::TitlebarContent)
        .tokens(shell_tokens(sub_h))
        .on_window_gesture(Message::WindowGesture)
        .maximized(store.chrome.maximized())
        .fullscreen(store.chrome.fullscreen())
        .breakpoints(store.themes.theme().breakpoints)
        .titlebar(move || crate::ui::topbar::titlebar(store, width));
    // A wired zero-height slot still registers a blank band.
    if mode.nav_is_stacked() {
        shell = shell.subtitlebar(move || {
            crate::ui::topbar::stacked_nav_row(route, &stacked_activity, width)
        });
    }
    shell.into_element()
}
