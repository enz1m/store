//! Native Store with per-user Flatpak operations and permission review.

#![deny(unconditional_recursion)]

mod i18n;

mod bounded_image;

mod catalog;

#[cfg(test)]
mod catalog_tests;
#[cfg(feature = "center")]
mod center;

mod featured_image;

mod flatpak;
mod flatpak_args;
#[cfg(test)]
mod flatpak_failure_tests;
#[cfg(test)]
mod flatpak_injection_tests;
mod install_consent;
#[cfg(test)]
mod install_flow;

mod modal_focus;
mod model;

mod shell_activity;

mod projection;

mod roots;

#[path = "app_parts/launch.rs"]
mod app_launch;
#[path = "app_parts/app_state.rs"]
mod app_state;

#[path = "app_parts/message.rs"]
mod message;
#[path = "app_parts/reduce_batch1.rs"]
mod reduce_batch1;
#[path = "app_parts/reduce_batch2.rs"]
mod reduce_batch2;
#[path = "app_parts/reduce_batch3.rs"]
mod reduce_batch3;
#[path = "app_parts/reduce_batch4.rs"]
mod reduce_batch4;
#[path = "app_parts/reduce_batch5.rs"]
mod reduce_batch5;
mod services;
#[path = "app_parts/shell.rs"]
mod shell;
pub(crate) use app_state::Store;
#[path = "app_parts/accessors.rs"]
mod accessors;
#[path = "app_parts/boot.rs"]
mod boot;
#[path = "app_parts/catalog_entries.rs"]
mod catalog_entries;
#[path = "app_parts/collections.rs"]
mod collections;
#[path = "app_parts/featured.rs"]
mod featured;
#[path = "app_parts/generations.rs"]
mod generations;
#[path = "app_parts/operations.rs"]
mod operations;
#[path = "app_parts/owned_reads.rs"]
mod owned_reads;
#[path = "app_parts/reduce.rs"]
mod reduce;
#[path = "app_parts/route.rs"]
mod route;
#[path = "app_parts/row_views.rs"]
mod row_views;
#[path = "app_parts/startup_load.rs"]
mod startup_load;
#[path = "app_parts/support.rs"]
mod support;
#[path = "app_parts/updates.rs"]
mod updates;
pub(crate) use message::Message;
pub(crate) use route::Route;
pub(crate) use startup_load::{
    CatalogCachePhase, fixture_settled_env, load_catalog_cache_phase, parse_args,
    refresh_catalog_from, settle_detail_fixture_now, settle_fixture_now,
};
#[cfg(test)]
pub(crate) use support::test_theme_integration;
pub(crate) use support::{
    CANCELLING_MESSAGE, CATALOG_QUEUE_CAPACITY, CATALOG_WORKER_COUNT, CLOSE_HELPER_CUSTODY_LIMIT,
    CLOSE_OWNER_WAIT, CLOSE_RECOVERY_PANICKED, CLOSE_RECOVERY_UNKNOWN, CloseDrainProbe,
    CloseDrainReport, CloseDrainSession, CloseHelperStatus, CloseOwners,
    DETAIL_GENERATION_EXHAUSTED, DETAIL_QUEUE_CAPACITY, DETAIL_WORKER_COUNT, DeferredOwnedReads,
    FEATURED_IMAGE_UNAVAILABLE, FEATURED_RETRY_INTERVAL, OPERATION_ACTIVITY_EXHAUSTED,
    OPERATION_ATTEMPT_EXHAUSTED, OPERATION_BUSY, OPERATION_CONCURRENCY_LIMIT,
    OPERATION_POLL_INTERVAL, OPERATION_QUEUE_CAPACITY, OperationCompletion, OwnedReadIntent,
    PermissionRead, PermissionTogglesState, READ_OWNER_RETRY_INTERVAL, READ_POOL_REMOTE_REFUSAL,
    READ_QUEUE_CAPACITY, READ_WORKER_COUNT, StoreExportOwner, blocking_owner,
    close_drain_start_failed, close_is_authorized, default_remotes, default_remotes_load,
    operation_for_in, permission_override_finished, permission_reset_finished, read_pool_task,
    read_terminal_text, start_close_drain,
};
mod theme;
mod ui;
mod url_launch;

use model::{AddRemoteForm, LibrarySort, OpKind, OperationState, SearchFilters};
use services::{AppEntry, RemoteInfo};

use enzui_core::focus::{Traversal, traversal_operation as traverse};
use enzui_core::operation_activity::{
    Activity, ActivityCenter, ActivityControl, ActivityKey, ActivityKind, ActivityProgress,
    ActivitySchedule, Generation, ProviderId, PublishOutcome, TerminalOutcome,
};
use enzui_core::theme_file::integration::{ThemeEvent, ThemeIntegration};
use enzui_theme::EnzTheme;
use enzui_widgets::app_shell::{ShellLayout, enz_window_settings};
use enzui_widgets::cached_subtree::{SubtreeHandle, SubtreeId};
use enzui_widgets::confirm_dialog::{ConfirmDialogEvent, enz_confirm_dialog};
use enzui_widgets::enz_responsive::enz_responsive;
use iced::{Element, Subscription, Task, advanced::widget::operate};

const STORE_ACTIVITY_PROVIDER: ProviderId = ProviderId("store");

fn classify_event(event: &iced::Event, status: iced::event::Status) -> Option<Message> {
    enzui_core::focus::traversal_from_event(event, status).map(|traversal| match traversal {
        Traversal::Next => Message::FocusNext,
        Traversal::Previous => Message::FocusPrevious,
    })
}

// Classify Ctrl+F as [`Message::FocusSearch`] on every route.
//
// Status is not consulted: a focused text field reports the chord captured,
// and Ctrl+F from the Library filter should still reach catalogue search. An
// open modal is refused in the reducer, which knows about modals.
fn classify_shortcut(event: &iced::Event) -> Option<Message> {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(c),
            modifiers,
            ..
        }) if modifiers.command()
            && !modifiers.shift()
            && !modifiers.alt()
            && c.eq_ignore_ascii_case("f") =>
        {
            Some(Message::FocusSearch)
        }
        _ => None,
    }
}

// A modal that captures Escape must not also trigger route navigation.
#[must_use]
fn classify_escape_back(event: &iced::Event, status: iced::event::Status) -> Option<Message> {
    if !matches!(status, iced::event::Status::Ignored) {
        return None;
    }
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(Message::Back),
        _ => None,
    }
}

// listen_with needs a function pointer, so route-specific subscriptions use separate entry points.
fn classify_event_on_detail(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    if matches!(
        event,
        iced::Event::Window(iced::window::Event::CloseRequested)
    ) {
        return Some(Message::CloseRequested(id));
    }
    if let Some(m) = classify_event(&event, status) {
        return Some(m);
    }
    if let Some(m) = classify_shortcut(&event) {
        return Some(m);
    }
    classify_escape_back(&event, status)
}

fn classify_event_off_detail(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    if matches!(
        event,
        iced::Event::Window(iced::window::Event::CloseRequested)
    ) {
        Some(Message::CloseRequested(id))
    } else {
        classify_event(&event, status).or_else(|| classify_shortcut(&event))
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScreenshotLoadState {
    #[default]
    Loading,
    KnownEmpty,
    Complete,
    Partial,
    Failed,
    CachedStale,
}

// Stable scrollable IDs let route changes reset widget-owned offsets through real scroll operations.
pub(crate) const LIBRARY_LIST_ID: &str = "store-library-list";
pub(crate) const SEARCH_LIST_ID: &str = "store-search-list";
pub(crate) const CATEGORY_LIST_ID: &str = "store-category-list";
pub(crate) const DETAIL_SCROLL_ID: &str = "store-detail-scroll";

impl Store {
    /// The theme used by the detail page and permissions dialog.
    pub(crate) fn theme_ref(&self) -> &EnzTheme {
        self.themes.theme()
    }

    // The operation this app's action slot must show, if any.
    //
    // The last matching operation wins: a retry pushes a second entry beside
    // the failed one, and the newer state is the current one.
    pub(crate) fn operation_for(&self, app_id: &str) -> Option<&model::OperationState> {
        operation_for_in(&self.operations, app_id)
    }

    /// Whether an app id is in the installed list — drives the detail page's
    /// install/remove control.
    pub(crate) fn is_installed(&self, app_id: &str) -> bool {
        self.installed.iter().any(|e| e.id == app_id)
    }

    /// Whether an installed app has a pending update.
    pub(crate) fn has_update(&self, app_id: &str) -> bool {
        self.update_for(app_id).is_some()
    }

    pub(crate) fn update_check_in_flight(&self) -> bool {
        match &self.update_discovery {
            services::UpdateDiscovery::Loading => true,
            services::UpdateDiscovery::Failed {
                retry_generation, ..
            } => self.updates_generation > *retry_generation,
            services::UpdateDiscovery::Known { .. } => false,
        }
    }

    fn activity_now_ms(&self) -> u64 {
        u64::try_from(self.activity_clock.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn subscription(&self) -> Subscription<Message> {
        self.build_subscription()
    }

    fn view(&self) -> Element<'_, Message> {
        i18n::localizer().view(|| {
            // Toast semantics stay outside the page's inert modal backgrounds.
            enzui_widgets::toast_host::enz_toast_layer(
                self.render_page(),
                self.notification_view(),
                enzui_widgets::toast_host::ToastOverlayPlacement::default(),
            )
        })
    }

    fn theme(&self) -> iced::Theme {
        self.theme_ref().iced_theme()
    }

    fn title(&self) -> String {
        i18n::label("Store").to_owned()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.detail_cancellation.cancel();
        for cancellation in self.operation_cancels.values() {
            cancellation.cancel();
        }
        self.catalog_owner.shutdown();
        if let Some(consent) = self.install_consent.take() {
            consent.cancellation.cancel();
        }
        self.detail_owner.shutdown();
        self.read_owner.shutdown();
        self.mutation_owner.shutdown();
        self.export_owner.shutdown();
    }
}

// ---------------------------------------------------------------------------
// CLI arguments (verification only)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> iced::Result {
    enzui_core::crash_report::CrashReporter::for_app("store-app", env!("CARGO_PKG_VERSION"));
    i18n::localizer().scope(run_store)
}

fn run_store() -> iced::Result {
    use enzui_widgets::single_instance::{InstanceStart, SingleInstance};
    let instance = match SingleInstance::start(DESKTOP_APP_ID) {
        InstanceStart::Owner(instance, _) => instance,
        InstanceStart::Forwarded => return Ok(()),
    };
    let instance = std::cell::RefCell::new(Some(instance));
    enzui_widgets::app_setup::run_with_services(
        iced::application(
            move || {
                let (mut store, boot) = Store::boot();
                if let Some(instance) = instance.borrow_mut().take() {
                    store.instance = instance;
                }
                (store, boot)
            },
            Store::update,
            Store::view,
        )
        .title(Store::title)
        .theme(Store::theme)
        .subscription(Store::subscription)
        // Derive decorations from the rendered shell layout to avoid a second titlebar.
        .window(with_desktop_identity(iced::window::Settings {
            size: DEFAULT_WINDOW_SIZE,
            ..enz_window_settings(ShellLayout::TitlebarContent)
        }))
        .exit_on_close_request(enzui_core::close_guard::EXIT_ON_CLOSE_REQUEST),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(i18n::localizer().clone())
            .traversal(enzui_widgets::app_setup::TraversalBinding::Disabled),
    )
}

#[cfg(test)]
mod test_scratch;
#[cfg(test)]
mod tests;

/// The window Store opens at: the narrowest at which Discover's sections
/// show two columns (`ui::discover::TWO_COLUMN_MIN_WIDTH` plus the page
/// insets), and tall enough for the featured banner and a section under it.
const DEFAULT_WINDOW_SIZE: iced::Size = iced::Size::new(904.0, 760.0);

// Match the shipped desktop basename so compositor grouping and application identity agree.
const DESKTOP_APP_ID: &str = "io.github.enzui.Store";

fn with_desktop_identity(settings: iced::window::Settings) -> iced::window::Settings {
    enzui_widgets::app_shell::DesktopApplicationIdentity::new(DESKTOP_APP_ID)
        .expect("BUG: DESKTOP_APP_ID must be a portable desktop-entry basename")
        .apply_to_window_settings(settings)
}

#[cfg(test)]
use startup_load::{load_catalog_cached, refresh_catalog_with_source};
