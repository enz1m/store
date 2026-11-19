//! Publisher-link launching with Store-specific scheme policy.
//!
//! Only HTTP, HTTPS and mailto links are accepted; validated URI bytes reach
//! the framework launcher unchanged. Supersession and app shutdown cancel the
//! pending request, while navigation preserves an explicit launch.

use enzui_core::portal::cancel::PortalCancel;
use enzui_core::portal::open_uri::{
    OpenUri, OpenUriBackend, OpenUriParseError, OpenUriRequest, PendingOpenUri,
};
use enzui_core::portal::{CancelReason, PortalOutcome, PortalParent};
use enzui_parent_lease::{ParentLease, ReleaseCause, RequestGeneration, SurfaceIncarnation};
use enzui_wayland::ParentExport;
use std::time::Duration;

/// The typed lifecycle outcome. Re-exported so the reducer's
/// `Message::OpenUrlFinished` arm has a single stable name.
pub(crate) type UrlLaunchOutcome = PortalOutcome<()>;

// ---------------------------------------------------------------------------
// Visible outcome text
// ---------------------------------------------------------------------------

/// The URL bytes did not parse as an RFC 3986 URI.
pub(crate) const OPEN_URL_BAD_SYNTAX: &str = "That link isn't a valid web address.";
/// The scheme is not one of Store's allow-listed schemes.
pub(crate) const OPEN_URL_BAD_SCHEME: &str = "Store only opens http, https, or mailto links.";
/// The link opener is not reachable on this host.
pub(crate) const OPEN_URL_UNAVAILABLE: &str = "Link opener is not available.";
/// The portal is present but does not implement OpenURI.
pub(crate) const OPEN_URL_UNSUPPORTED: &str = "This desktop can't open links via portal.";
/// The manifest does not declare the OpenURI capability.
pub(crate) const OPEN_URL_NOT_DECLARED: &str = "Opening links is not enabled for this build.";
/// The user (or the portal) explicitly refused permission.
pub(crate) const OPEN_URL_DENIED: &str = "The system refused to open this link.";
/// The parent window handle for the request could not be resolved.
pub(crate) const OPEN_URL_PARENT_UNAVAILABLE: &str =
    "Couldn't attach the link opener to this window.";
/// A transport / bus / unspecified failure.
pub(crate) const OPEN_URL_FAILED: &str = "Could not open link.";
/// The user dismissed the portal handler dialog.
pub(crate) const OPEN_URL_USER_CANCELLED: &str = "Open cancelled.";

/// Map a typed [`UrlLaunchOutcome`] to the visible line on
/// `Store::last_url_error`, or `None` when the outcome is silent (success,
/// or an application/parent-driven cancellation).
///
/// Every [`PortalOutcome`] arm is named so a new variant is a compile
/// error here, never a silently generic "failed".
pub(crate) fn outcome_text(outcome: &UrlLaunchOutcome) -> Option<String> {
    let s = match outcome {
        PortalOutcome::Completed(()) => return None,
        PortalOutcome::Cancelled(CancelReason::User) => OPEN_URL_USER_CANCELLED,
        PortalOutcome::Cancelled(CancelReason::Application | CancelReason::ParentClosed) => {
            return None;
        }
        PortalOutcome::Denied => OPEN_URL_DENIED,
        PortalOutcome::NotDeclared => OPEN_URL_NOT_DECLARED,
        PortalOutcome::Unsupported => OPEN_URL_UNSUPPORTED,
        PortalOutcome::Unavailable => OPEN_URL_UNAVAILABLE,
        PortalOutcome::ParentUnavailable => OPEN_URL_PARENT_UNAVAILABLE,
        PortalOutcome::Failed => OPEN_URL_FAILED,
    };
    Some(s.to_owned())
}

/// The visible line for a bounded parent-export pool refusal.
///
/// A pool refusal — a full queue, a cancelled request, or a stopped or panicked
/// worker — is a transient failure to prepare the parent, not a proven absence
/// of one. It surfaces a typed line rather than a silent unparented open.
pub(crate) fn parent_export_refusal_text(
    error: enzui_core::blocking_task::BlockingTaskError,
) -> String {
    use enzui_core::blocking_task::BlockingTaskError;
    let s = match error {
        BlockingTaskError::Cancelled => OPEN_URL_USER_CANCELLED,
        // Busy, a stopped or panicked worker, or any future terminal error:
        // fail closed to a visible parent-unavailable line.
        _ => OPEN_URL_PARENT_UNAVAILABLE,
    };
    s.to_owned()
}

/// Store's scheme allow-list, compared against the parsed scheme with
/// [`str::eq_ignore_ascii_case`] rather than a `starts_with` prefix.
///
/// RFC 3986 §3.1 makes scheme names case-insensitive on the wire, so
/// `HTTPS:`, `MailTo:`, etc. must be accepted; the URI bytes themselves
/// are still passed through verbatim.
pub(crate) const ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto"];

// Parse a raw URL and enforce Store's scheme allow-list without
// modifying the input bytes.
//
// This is the ONE URI parser Store owns; the reducer must not call
// [`OpenUri::parse`] separately. Returns `Ok(uri)` only when the URL is
// RFC 3986 syntax AND the parsed scheme case-insensitively equals one of
// [`ALLOWED_SCHEMES`]. The returned [`OpenUri::as_str`] equals the input
// bytes byte-for-byte.
fn parse_and_screen(raw: &str) -> Result<OpenUri, ParseFailure> {
    let uri = OpenUri::parse(raw.to_owned()).map_err(ParseFailure::Syntax)?;
    let scheme = uri
        .as_str()
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .expect("OpenUri::parse guarantees a `scheme:` prefix");
    if !ALLOWED_SCHEMES
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(scheme))
    {
        return Err(ParseFailure::Scheme);
    }
    Ok(uri)
}

/// Why a URL was refused before any wire request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParseFailure {
    /// The URL is not RFC 3986 URI syntax.
    Syntax(OpenUriParseError),
    /// The scheme is not in Store's allow-list.
    Scheme,
}

impl ParseFailure {
    /// Visible line for this parse rejection.
    #[must_use]
    pub(crate) fn text(&self) -> &'static str {
        match self {
            Self::Syntax(_) => OPEN_URL_BAD_SYNTAX,
            Self::Scheme => OPEN_URL_BAD_SCHEME,
        }
    }
}

/// Why [`UrlLauncher::start`] refused a request before or during dispatch.
///
/// The reducer's single finish path relies on this being the ONE type
/// returned by `start` — a typed [`ParseFailure`] cannot be collapsed to
/// generic `PortalOutcome::Failed` by a downstream caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartOutcome {
    /// Store's parse+scheme policy refused the URL before any wire work.
    Parse(ParseFailure),
    /// The framework transport reported a terminal outcome before or
    /// during dispatch.
    Portal(UrlLaunchOutcome),
}

// ---------------------------------------------------------------------------
// UrlLauncher — the owned lifecycle
// ---------------------------------------------------------------------------

/// The store-app link-opener state.
///
/// Owns at most one live [`PendingOpenUri`] at a time. Starting a second
/// URL supersedes the first: the previous request is cancelled with
/// [`CancelReason::Application`] and its close obligation is settled
/// against the same backend before the new request begins, so no orphan
/// remains once a new open reaches the reducer.
pub(crate) struct UrlLauncher {
    /// Built on first use so merely constructing the app spawns no tokio
    /// runtime (render tests never open a publisher chip).
    backend: Option<Box<dyn OpenUriBackend>>,
    /// The live request, if any. Cleared on completion so a stale reply
    /// cannot be observed twice.
    pending: Option<PendingOpenUri>,
    /// Monotone request generation, bumped whenever a new request is
    /// installed. Stamped into the completion subscription so a stale
    /// queued wake from a superseded request cannot pump the current one.
    generation: u64,
    /// Parent export retained for the live request.
    parent: ParentLease<ParentExport>,
    /// Cancellation shared with an in-flight parent export.
    parent_cancel: PortalCancel,
}

/// Maximum time allowed for the xdg-foreign export round trip.
pub(crate) const PARENT_EXPORT_TIMEOUT: Duration = Duration::from_millis(750);

impl std::fmt::Debug for UrlLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UrlLauncher")
            .field(
                "backend",
                &self.backend.as_ref().map(|b| b.describe_open_uri()),
            )
            .field("has_pending", &self.pending.is_some())
            .field("generation", &self.generation)
            .finish()
    }
}

impl Default for UrlLauncher {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlLauncher {
    /// A launcher with no backend built and nothing in flight.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            backend: None,
            pending: None,
            generation: 0,
            parent: ParentLease::new(),
            parent_cancel: PortalCancel::new(),
        }
    }

    /// A launcher driven by an injected backend — the seam the reducer
    /// tests use to play success / cancel / denial in-process, with no
    /// session bus and no dialog on the developer's desktop.
    #[cfg(test)]
    pub(crate) fn with_backend(backend: Box<dyn OpenUriBackend>) -> Self {
        Self {
            backend: Some(backend),
            pending: None,
            generation: 0,
            parent: ParentLease::new(),
            parent_cancel: PortalCancel::new(),
        }
    }

    /// The current live request generation (monotone). A wake carrying a
    /// smaller value is stale and must not drive a pump.
    #[must_use]
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Start opening `href` after Store's single parse + scheme screen.
    ///
    /// Returns `Ok(())` when a request is now pending, and
    /// `Err(StartOutcome)` for a request that never started — the URI
    /// failed Store's screen, no portal is reachable, the manifest forbids
    /// it, or a stale parent. Both branches route through the same
    /// completion path so the reducer has one finish arm.
    pub(crate) fn start(&mut self, href: &str) -> Result<(), StartOutcome> {
        let uri = parse_and_screen(href).map_err(StartOutcome::Parse)?;
        // Supersede a live request before installing the new one, so a
        // second click never leaks the first generation. Both cancel and
        // settle are non-blocking by the backend contract.
        self.cancel_and_settle(CancelReason::Application);
        let Some(backend) = self.backend_mut() else {
            return Err(StartOutcome::Portal(PortalOutcome::Unavailable));
        };
        let backend: &mut dyn OpenUriBackend = &mut **backend;
        // `PortalParent::None` resolves to `Unparented` regardless of the
        // predicate. Store adopts an exported window handle in a follow-up;
        // meanwhile a constant is honest rather than a fake window lookup.
        match OpenUriRequest::new(uri, PortalParent::None).start(backend, |_| true) {
            Ok(pending) => {
                self.pending = Some(pending);
                self.generation = self.generation.wrapping_add(1);
                Ok(())
            }
            Err(outcome) => Err(StartOutcome::Portal(outcome)),
        }
    }

    /// Starts opening `href` with an exported parent window.
    ///
    /// The export is retained until settlement. A stale parent returns
    /// [`PortalOutcome::ParentUnavailable`] before backend dispatch.
    pub(crate) fn start_parented(
        &mut self,
        href: &str,
        window: iced::window::Id,
        export: ParentExport,
        surface: SurfaceIncarnation,
        is_live: impl Fn(iced::window::Id) -> bool,
    ) -> Result<(), StartOutcome> {
        let uri = parse_and_screen(href).map_err(StartOutcome::Parse)?;
        self.cancel_and_settle(CancelReason::Application);
        let Some(backend) = self.backend_mut() else {
            return Err(StartOutcome::Portal(PortalOutcome::Unavailable));
        };
        let backend: &mut dyn OpenUriBackend = &mut **backend;
        let parent_handle = export.parent_handle();
        match OpenUriRequest::new(uri, PortalParent::Window(window))
            .with_parent_handle(parent_handle)
            .start(backend, is_live)
        {
            Ok(pending) => {
                self.pending = Some(pending);
                self.generation = self.generation.wrapping_add(1);
                let generation = RequestGeneration(self.generation);
                if let Some(superseded) = self.parent.install(export, surface, generation) {
                    let mut old = superseded.lease;
                    old.release();
                }
                Ok(())
            }
            Err(outcome) => {
                let mut refused = export;
                refused.release();
                Err(StartOutcome::Portal(outcome))
            }
        }
    }

    /// The event-driven wake source for the live request, stamped with the
    /// launcher's request generation so a stale queued wake from a
    /// superseded generation can be rejected before it pumps the current
    /// one.
    ///
    /// [`iced::Subscription::none`] unless a request is actually pending
    /// AND the backend has an asynchronous side, so a settled app holds
    /// no portal wake source at all.
    pub(crate) fn completion_subscription(&self) -> iced::Subscription<u64> {
        let Some(pending) = self.pending.as_ref() else {
            return iced::Subscription::none();
        };
        // Gate on `is_unsettled` rather than `is_pending`: an external
        // cancel — including one through a cloned `PortalCancel` — flips
        // the request terminal *before* its exactly-once close obligation
        // has been claimed. A subscription gated on `is_pending` would
        // vanish in that window, so a subsequent iced rebuild would find
        // no wake source and the settle would never run. `is_unsettled`
        // stays true until the close is claimed by `settle`, keeping the
        // same recipe alive so the wake can route the owed close.
        if !pending.is_unsettled() {
            return iced::Subscription::none();
        }
        let Some(signal) = self
            .backend
            .as_deref()
            .and_then(OpenUriBackend::open_uri_completion_signal)
        else {
            return iced::Subscription::none();
        };
        let generation = self.generation;
        enzui_widgets::file_chooser::completion_signal_subscription(generation, signal)
            .with(generation)
            .map(|(generation, _)| generation)
    }

    // Settle the owed close before clearing a completed request; cancellation may win during poll.
    pub(crate) fn pump(&mut self) -> Option<UrlLaunchOutcome> {
        let pending = self.pending.as_mut()?;
        let backend = self
            .backend
            .as_deref_mut()
            .expect("a pending request implies a backend was built");

        // Poll first: the common case is a still-live request returning
        // None with nothing owed.
        let mut outcome = pending.poll(backend);

        // Adopt a terminal state recorded by an off-thread cancel that
        // ended the request without a reply. `poll` returns None for an
        // already-terminal request (a late reply must not overwrite the
        // cancel), so `cancel_reason()` is the only way `pump` observes it.
        if outcome.is_none()
            && let Some(reason) = pending.cancel_reason()
        {
            outcome = Some(PortalOutcome::Cancelled(reason));
        }

        // Perform any owed close unconditionally, before releasing the
        // slot. This is what carries the wire Close for a cancel that won
        // during poll (poll already returned `Some(Cancelled)` above) as
        // well as for the poll-None adoption branch.
        let _ = pending.settle(backend);

        if outcome.is_some() {
            self.pending = None;
        }
        outcome
    }

    /// Cancels and settles the exact current generation, if any, before
    /// releasing it. Used on supersession and on [`Drop`].
    pub(crate) fn cancel_and_settle(&mut self, reason: CancelReason) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        let _ = pending.cancel(reason);
        if let Some(backend) = self.backend.as_deref_mut() {
            let _ = pending.settle(backend);
        }
        let cause = match reason {
            CancelReason::ParentClosed => ReleaseCause::WindowClosed,
            _ => ReleaseCause::Cancelled,
        };
        self.release_parent(cause);
    }

    /// Returns the cancellation shared with the next parent export.
    #[must_use]
    pub(crate) fn parent_cancel(&self) -> PortalCancel {
        self.parent_cancel.clone()
    }

    /// Releases the held parent export, if any.
    pub(crate) fn release_parent(&mut self, cause: ReleaseCause) {
        if let Some(released) = self.parent.release(cause) {
            let mut export = released.lease;
            export.release();
        }
    }

    // Route lazily; native applications use the user's default handler and a
    // sandbox keeps the portal boundary.
    fn backend_mut(&mut self) -> Option<&mut Box<dyn OpenUriBackend>> {
        if self.backend.is_none() {
            self.backend =
                Some(enzui_core::portal::open_uri::desktop_launcher::host_open_uri_backend());
        }
        self.backend.as_mut()
    }
}

impl Drop for UrlLauncher {
    fn drop(&mut self) {
        // Non-blocking by the backend contract, so this is safe on any
        // drop path. Records `Application` (not `ParentClosed`) because
        // Store's drop is app teardown — the parent window is going down
        // WITH the state, not out from under it.
        //
        // The framework backend drains owed closes during bounded shutdown.
        self.cancel_and_settle(CancelReason::Application);
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// A `Send`-safe wrapper that lets a test hold an
/// `Arc<Mutex<MockOpenUriBackend>>` AND hand a `Box<dyn OpenUriBackend>`
/// to [`UrlLauncher::with_backend`], so the same test can script replies
/// AFTER the reducer has installed the pending request (whose generated
/// [`RequestId`] the caller then knows).
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    use enzui_core::portal::backend::{BackendError, RequestId};
    use enzui_core::portal::completion::CompletionSignal;
    use enzui_core::portal::open_uri::{MockOpenUriBackend, OpenUriBackend, OpenUriReply};
    use enzui_core::portal::parent::ParentHandle;

    #[derive(Clone)]
    pub(crate) struct SharedMockBackend {
        inner: Arc<Mutex<MockOpenUriBackend>>,
        signal: CompletionSignal,
    }

    impl SharedMockBackend {
        pub(crate) fn new() -> Self {
            Self {
                inner: Arc::new(Mutex::new(MockOpenUriBackend::new())),
                signal: CompletionSignal::new(),
            }
        }

        pub(crate) fn boxed(&self) -> Box<dyn OpenUriBackend> {
            Box::new(self.clone())
        }

        pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, MockOpenUriBackend> {
            self.inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    impl OpenUriBackend for SharedMockBackend {
        fn start_open_uri(
            &mut self,
            uri: &OpenUri,
            parent: &ParentHandle,
        ) -> Result<RequestId, BackendError> {
            self.lock().start_open_uri(uri, parent)
        }
        fn poll_open_uri(&mut self, id: RequestId) -> Option<OpenUriReply> {
            self.lock().poll_open_uri(id)
        }
        fn close_open_uri(&mut self, id: RequestId, reason: CancelReason) {
            self.lock().close_open_uri(id, reason);
        }
        fn open_uri_completion_signal(&self) -> Option<CompletionSignal> {
            Some(self.signal.clone())
        }
        fn describe_open_uri(&self) -> String {
            self.lock().describe_open_uri()
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn start_rejects_prefix_lookalikes_and_bad_schemes() {
        for reject in [
            "httpsx://evil",       // scheme lookalike
            "javascript:alert(1)", // scripted URL scheme
            "file:///etc/passwd",  // filesystem scheme
            "ftp://example.com",   // FTP is not on the allow-list
            "chrome://settings",   // internal handler
            "data:text/plain,hi",  // data URL
        ] {
            let mock = tests::SharedMockBackend::new();
            let mut launcher = UrlLauncher::with_backend(mock.boxed());
            assert!(
                matches!(
                    launcher.start(reject),
                    Err(StartOutcome::Parse(ParseFailure::Scheme))
                ),
                "must be a scheme rejection: {reject:?} => {:?}",
                launcher.start(reject)
            );
            assert!(
                mock.lock().started.is_empty(),
                "no scheme rejection may reach the backend: {reject:?}"
            );
        }
    }
}
