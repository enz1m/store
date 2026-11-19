//! Detail, screenshot, and toggle accessors.

use super::*;

impl Store {
    /// The app id the detail page is showing, if any.
    pub(crate) fn detail_app_id_ref(&self) -> Option<&str> {
        self.detail_app_id.as_deref()
    }

    /// The live add-remote form, if the dialog is open (cluster 1).
    pub(crate) fn add_remote_form_ref(&self) -> Option<&AddRemoteForm> {
        self.add_remote_form.as_ref()
    }

    /// The permissions loaded for the detail page, if the read succeeded.
    ///
    /// `None` for both the in-flight read and a failed one, so neither a load
    /// still in progress nor a failure is ever rendered as a declared set.
    pub(crate) fn detail_permissions(&self) -> Option<&services::Permissions> {
        match &self.detail_permissions {
            PermissionRead::Loaded(perms) => Some(perms),
            PermissionRead::Loading | PermissionRead::Unavailable => None,
        }
    }

    /// Whether the detail-page permission read failed and can be retried. A
    /// read still in flight is not unavailable, so the retry surfaces only on a
    /// real failure.
    pub(crate) fn detail_permissions_unavailable(&self) -> bool {
        matches!(self.detail_permissions, PermissionRead::Unavailable)
    }

    /// The loaded detail-page enrichment (releases, links), if the read has
    /// completed (Cluster A).
    pub(crate) fn detail_context(&self) -> Option<&services::DetailContext> {
        self.detail_context.as_ref()
    }

    pub(crate) fn detail_request_error(&self) -> Option<&'static str> {
        self.detail_request_error
    }

    pub(crate) fn close_failure_text(&self) -> Option<String> {
        let report = self.close_drain_report?;
        if self.close_helper_status == CloseHelperStatus::Panicked {
            return Some(
            "The close helper ended unexpectedly. Its join result is uncertain, so Store will remain open.".to_owned(),
        );
        }
        if self.close_helper_status == CloseHelperStatus::ReaperDebt {
            return Some(
            "The close helper exceeded its bounded drain and remains under retained reaper custody. Store will remain open.".to_owned(),
        );
        }
        if report.completed() {
            if self.close_helper_status == CloseHelperStatus::Running
                && self
                    .close_started
                    .is_some_and(|started| started.elapsed() >= CLOSE_OWNER_WAIT)
            {
                return Some(
                "All owners reported joined, but the exact close helper has not exited yet. Store will remain open.".to_owned(),
            );
            }
            return None;
        }
        if report.start_failed() {
            return Some(
            "Store could not start the background-work drain. Its exact owners are retained; retry or keep the window open.".to_owned(),
        );
        }
        if report.has_panicked() {
            return Some(
            "A background worker ended unexpectedly. Its result is unknown, so Store will not claim that close completed.".to_owned(),
        );
        }
        if report.has_timed_out() {
            return Some(format!(
                "{} background worker(s) are still under exact join custody. Retry rechecks their join results; Keep Open waits before restoring service.",
                report.affected_workers()
            ));
        }
        None
    }

    // The selected featured app's screenshot lifecycle. An unseen page starts
    // in `Loading`; its fixed-size view paints a named placeholder until the
    // background task resolves.
    pub(crate) fn featured_image_state(
        &self,
        app_id: &str,
    ) -> enzui_core::async_image::ImageState<iced::widget::image::Handle> {
        self.featured_images
            .get(app_id)
            .cloned()
            .unwrap_or(featured_image::HeroImageState::Loading)
            .image_state()
    }

    /// A featured app's brand colours (light, dark), once its banner loaded.
    pub(crate) fn featured_brand(&self, app_id: &str) -> Option<([u8; 3], [u8; 3])> {
        self.featured_brands.get(app_id).copied()
    }

    /// Cached detail screenshots; empty while loading or when none are published.
    pub(crate) fn screenshots(&self) -> &[services::Screenshot] {
        &self.screenshots
    }

    pub(crate) fn screenshot_viewer(&self) -> Option<usize> {
        self.screenshot_viewer
    }

    pub(crate) fn screenshot_load_state(&self) -> ScreenshotLoadState {
        self.screenshot_load_state
    }

    pub(crate) fn screenshot_strip_id(index: usize) -> iced::advanced::widget::Id {
        enzui_core::identity::derive_id("store-screenshot", &index.to_string())
    }

    /// The release history for the current detail app, newest first. Empty
    /// until the context read lands or when the app ships none.
    pub(crate) fn detail_releases(&self) -> &[services::FlathubRelease] {
        self.detail_context
            .as_ref()
            .map(|c| c.releases.as_slice())
            .unwrap_or(&[])
    }

    /// Whether the release-history dialog is open (Cluster A).
    pub(crate) fn release_history_open(&self) -> bool {
        self.release_history_open
    }

    /// The tri-state permission toggles for the current detail app (Cluster B).
    pub(crate) fn permission_toggles(&self) -> &[services::PermissionToggle] {
        match &self.permission_toggles {
            PermissionTogglesState::Ready(toggles) => toggles,
            PermissionTogglesState::Loading | PermissionTogglesState::Unavailable => &[],
        }
    }

    /// Whether the Sandbox Permissions model could not be built. Mutation
    /// controls stay disabled and a retry is offered in this state.
    pub(crate) fn permission_toggles_unavailable(&self) -> bool {
        matches!(self.permission_toggles, PermissionTogglesState::Unavailable)
    }

    /// Whether the editable model is authoritative, so mutation controls may
    /// act. A load still in flight is not ready.
    pub(crate) fn permission_toggles_ready(&self) -> bool {
        matches!(self.permission_toggles, PermissionTogglesState::Ready(_))
    }

    // The editable model, mutable only when it is authoritative.
    pub(crate) fn permission_toggles_mut(
        &mut self,
    ) -> Option<&mut Vec<services::PermissionToggle>> {
        match &mut self.permission_toggles {
            PermissionTogglesState::Ready(toggles) => Some(toggles),
            PermissionTogglesState::Loading | PermissionTogglesState::Unavailable => None,
        }
    }
}
