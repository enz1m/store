//! Bounded, authored translation of permission-policy write/reset failures.
//!
//! The injected [`PermissionOverrideRunner`](super::PermissionOverrideRunner)
//! returns a raw `Err(String)` that can carry hostile Flatpak stderr, backend,
//! or control payload (long paths, errno text, C0/C1 controls, bidi
//! overrides). That string must never reach the sandbox dialog as semantic or
//! accessibility text.
//!
//! At the runner boundary every failure is classified into a fixed, authored
//! category. [`PermissionActionFailure::message`] renders one bounded
//! error surface — action context plus a static category phrase, with no raw
//! detail. Raw detail survives only through [`PermissionActionFailure::diagnostic`],
//! an escaped diagnostic sink stripped of control/escape sequences and capped.

use super::flatpak_failure::{cap_chars, sanitize_diagnostic};

// Ceiling for the complete authored error surface.
pub(crate) const RENDERED_PERMISSION_FAILURE_CHAR_CAP: usize = 200;

// Ceiling for the escaped diagnostic sink.
pub(crate) const PERMISSION_DIAGNOSTIC_CHAR_CAP: usize = 200;

/// Which permission-policy operation failed. Distinguishes the write and reset
/// surfaces so their action context can never be confused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionAction {
    /// A single checked `flatpak override --user` write.
    Write,
    /// A whole-policy `flatpak override --user --reset`.
    Reset,
}

// Remove Unicode bidirectional and related format-control characters that
// `char::is_control` does not classify as control (RLO/LRO/isolates/marks).
fn strip_bidi(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(c,
                '\u{200e}' | '\u{200f}' | '\u{061c}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

// Escape an arbitrary raw string for a diagnostic sink or a bounded inline
// machine token: strip terminal-escape/control sequences and bidi/format
// overrides. The result is free of control and bidi characters.
pub(crate) fn escape_detail(raw: &str) -> String {
    strip_bidi(&sanitize_diagnostic(raw))
}

impl PermissionAction {
    fn phrase(self) -> &'static str {
        match self {
            Self::Write => "Could not save permission policy",
            Self::Reset => "Could not reset permission policy",
        }
    }
}

/// A fixed, authored category for a failed permission-policy operation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PermissionFailureKind {
    /// The application ID failed reverse-DNS validation before any write, so
    /// the runner was never called. The copy stays actionable but bounded.
    InvalidAppId,
    /// The injected runner (production: `flatpak override --user`) rejected the
    /// policy change. Raw backend detail is diverted to the diagnostic sink.
    Rejected,
    /// The bounded read pool refused the change before the runner ran, so
    /// nothing was written and the optimistic value is rolled back. Retrying
    /// the same toggle resubmits it.
    Unavailable,
}

impl PermissionFailureKind {
    fn phrase(&self) -> &'static str {
        match self {
            Self::InvalidAppId => "invalid Flatpak application ID",
            Self::Rejected => "the system rejected the change",
            Self::Unavailable => "the system was busy and the change was not attempted",
        }
    }
}

/// A bounded, authored permission-policy failure and its escaped diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PermissionActionFailure {
    action: PermissionAction,
    kind: PermissionFailureKind,
    diagnostic: Option<String>,
}

impl PermissionActionFailure {
    // The app ID failed validation before the runner was reached. No raw
    // backend payload exists, so there is no diagnostic sink.
    pub(crate) fn invalid_app_id(action: PermissionAction) -> Self {
        Self {
            action,
            kind: PermissionFailureKind::InvalidAppId,
            diagnostic: None,
        }
    }

    // The bounded read pool refused the write before the runner ran. There is
    // no backend payload, so there is no diagnostic sink; the optimistic value
    // is rolled back and the toggle stays retryable.
    pub(crate) fn unavailable(action: PermissionAction) -> Self {
        Self {
            action,
            kind: PermissionFailureKind::Unavailable,
            diagnostic: None,
        }
    }

    // The injected runner rejected the change. The raw error is classified to
    // a fixed category; its detail is escaped and diverted to the sink.
    pub(crate) fn rejected(action: PermissionAction, raw: &str) -> Self {
        // `sanitize_diagnostic` strips control and terminal-escape sequences;
        // bidi/format overrides are not `char::is_control`, so remove them too
        // before the detail may enter the escaped diagnostic sink.
        let clean = escape_detail(raw);
        let diagnostic =
            (!clean.is_empty()).then(|| cap_chars(&clean, PERMISSION_DIAGNOSTIC_CHAR_CAP));
        Self {
            action,
            kind: PermissionFailureKind::Rejected,
            diagnostic,
        }
    }

    // The single authored, control/bidi/path/errno-free error surface.
    //
    // Built only from static phrases, so no raw backend payload can enter the
    // visible or accessibility copy.
    pub(crate) fn message(&self) -> String {
        let message = format!("{} — {}.", self.action.phrase(), self.kind.phrase());
        cap_chars(&message, RENDERED_PERMISSION_FAILURE_CHAR_CAP)
    }

    // The escaped diagnostic sink. Raw backend detail reaches only here,
    // stripped of control/escape sequences and bounded.
    pub(crate) fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}
