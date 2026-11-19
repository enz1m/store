//! App Detail and permission surfaces.
//!
//! Declared permissions are read-only; checked user-policy edits roll back on
//! failure and pre-install edits disclose that their policy applies later.

use enzui_core::semantics::HeadingLevel;
use enzui_theme::EnzTheme;
use enzui_theme::spacing::SpaceToken;
use enzui_widgets::badge::{BadgeTone, enz_badge};
use enzui_widgets::button::ButtonSize;
use enzui_widgets::capped_dialog_body::CappedDialogBody;
use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};
use enzui_widgets::modal_host::enz_modal_host;
use enzui_widgets::modal_surface::{ModalSurfaceTokens, enz_modal_surface};
use enzui_widgets::motion::{MotionAxis, MotionEffect, motion};
use enzui_widgets::pressable::{PressableStyle, enz_pressable};
use enzui_widgets::progress_bar::enz_progress_bar;
use enzui_widgets::rating::enz_rating;
use enzui_widgets::scroll_view::{Axis, enz_scroll_view};
use enzui_widgets::surface::{BorderStyle, SurfaceTone, enz_surface};
use enzui_widgets::window_title::{WindowTitleTokens, enz_window_title};
use enzui_widgets::{ButtonKind, enz_button};
use iced::widget::{Space, container, row};
use iced::{Alignment, Element, Length};

use crate::services::Permissions;
use crate::{Message, Store};

use super::chrome;

/// A stable permissions invoker ID restores focus after dismissal; only one Detail page is live.
pub(crate) const PERMISSIONS_INVOKER_ID: &str = "store-detail-view-permissions";

// The `widget::Id` the invoking button carries and that focus is restored to.
// Stable scope key shared by the permissions lifecycle and host.
pub(crate) const PERMISSIONS_SCOPE_KEY: &str = "store-permissions-dialog";
pub(crate) const RELEASE_HISTORY_SCOPE_KEY: &str = "store-release-history";
pub(crate) const SANDBOX_SCOPE_KEY: &str = "store-sandbox-permissions";

#[path = "detail/about.rs"]
mod about;
#[path = "detail/labels.rs"]
mod labels;
#[path = "detail/permission_list.rs"]
mod permission_list;
#[path = "detail/pills.rs"]
mod pills;
#[path = "detail/present.rs"]
mod present;
pub(crate) use about::about_section;
pub(crate) use labels::{
    DIALOG_HEIGHT_FRACTION, loading_screenshots_copy, permission_glance, ratings_attribution,
    ratings_breakdown_toggle_label, update_all_label, whats_new_heading,
};
pub(crate) use pills::{
    DETAIL_ACTION_WIDTH, permission_pills, present_screenshot_viewer, screenshot_gallery,
    screenshot_gallery_placeholder, two_column,
};
pub(crate) use present::{dialog_width, present};
pub(crate) use releases::{present_release_history, release_entry};
pub(crate) use sandbox::present_sandbox_permissions;
pub(crate) use view_fns::{
    permissions_invoker_id, release_history_invoker_id, sandbox_invoker_id, view,
};
#[path = "detail/releases.rs"]
mod releases;
#[path = "detail/sandbox.rs"]
mod sandbox;
#[path = "detail/view_fns.rs"]
mod view_fns;
