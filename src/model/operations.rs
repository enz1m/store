//! Running flatpak operation state.

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// What a running flatpak operation is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpKind {
    Install,
    Uninstall,
    Update,
}

impl OpKind {
    /// Verb shown in the Activity row while the operation runs.
    pub(crate) fn verb(self) -> &'static str {
        match self {
            OpKind::Install => crate::i18n::label("Installing"),
            OpKind::Uninstall => crate::i18n::label("Removing"),
            OpKind::Update => crate::i18n::label("Updating"),
        }
    }

    /// Past-tense word shown once the operation finishes.
    pub(crate) fn past(self) -> &'static str {
        match self {
            OpKind::Install => crate::i18n::label("Installed"),
            OpKind::Uninstall => crate::i18n::label("Removed"),
            OpKind::Update => crate::i18n::label("Updated"),
        }
    }
}

/// One in-flight or just-finished flatpak operation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OperationState {
    /// A monotonic per-app attempt generation that prevents stale cancellation from reaching a retry.
    pub attempt: u64,
    pub app_id: String,
    pub name: String,
    pub kind: OpKind,
    /// `0.0..=1.0`.
    pub progress: f32,
    /// Whether `progress` holds a reported measurement or the starting
    /// placeholder.
    ///
    /// `flatpak` reports nothing until its first progress line, so a fresh
    /// operation sits at `progress: 0.0` without that being a measurement;
    /// the control renders an unsampled placeholder until this flips.
    pub sampled: bool,
    pub message: String,
    pub finished: bool,
    pub error: Option<String>,
    /// The cancellation stage of this attempt.
    pub cancel: CancelState,
}

/// Cancellation remains pending until the child group exits and both output readers join.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum CancelState {
    /// No cancellation asked for. Cancel is offered.
    #[default]
    Idle,
    /// Requested and signalled; the process group is being reaped. Cancel
    /// stays **visible but disabled** and the row reads "Cancelling…".
    ///
    /// Visible, not hidden: a control that disappears on press leaves a user
    /// unsure whether the press registered, and this row can sit here for the
    /// grace interval. Disabled, so a second press cannot be mistaken for a
    /// stronger one — the reducer is idempotent regardless.
    Cancelling,
    /// The operation settled as cancelled. Terminal.
    Cancelled,
}

impl CancelState {
    /// Whether a Cancel control may be rendered at all.
    ///
    /// True through `Cancelling`: the requirement is that Cancel is withdrawn
    /// on terminal **settlement**, not on signal dispatch.
    #[must_use]
    pub(crate) fn offers_cancel(self) -> bool {
        matches!(self, Self::Idle | Self::Cancelling)
    }

    /// Whether that control may be pressed.
    #[must_use]
    pub(crate) fn cancel_is_actionable(self) -> bool {
        matches!(self, Self::Idle)
    }
}

/// An app-and-attempt identity that confines cancellation to the exact operation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OperationId {
    pub app_id: String,
    pub attempt: u64,
}

impl OperationId {
    #[must_use]
    pub(crate) fn new(app_id: impl Into<String>, attempt: u64) -> Self {
        Self {
            app_id: app_id.into(),
            attempt,
        }
    }

    /// The accessible name for this attempt's Cancel control.
    ///
    /// App-specific because a page can show several in-flight rows at once,
    /// and a screen-reader user choosing between three buttons all called
    /// "Cancel" is choosing blind.
    #[must_use]
    pub(crate) fn cancel_aria_label(&self, verb: &str, name: &str) -> String {
        crate::i18n::format(
            crate::i18n::localizer(),
            "Cancel {} {name}",
            &[("arg0", &(verb.to_lowercase())), ("name", &(name))],
        )
    }
}

impl OperationState {
    pub(crate) fn starting(
        app_id: impl Into<String>,
        name: impl Into<String>,
        kind: OpKind,
    ) -> Self {
        Self::attempt(app_id, name, kind, 0)
    }

    /// A starting row with an explicit attempt generation.
    pub(crate) fn attempt(
        app_id: impl Into<String>,
        name: impl Into<String>,
        kind: OpKind,
        attempt: u64,
    ) -> Self {
        Self {
            attempt,
            app_id: app_id.into(),
            name: name.into(),
            kind,
            progress: 0.0,
            sampled: false,
            message: format!("{}…", kind.verb()),
            finished: false,
            error: None,
            cancel: CancelState::Idle,
        }
    }

    /// This row's stable operation identity.
    #[must_use]
    pub(crate) fn id(&self) -> OperationId {
        OperationId::new(&self.app_id, self.attempt)
    }

    /// Whether cancellation may target this live row.
    #[must_use]
    pub(crate) fn is_in_flight(&self) -> bool {
        !self.finished
    }

    /// This operation's completion in the sampled/unsampled vocabulary
    /// [`enzui_widgets::progress_bar`] takes.
    ///
    /// A finished operation reads as full regardless of its last sample:
    /// `flatpak` often exits without a closing 100% line.
    pub(crate) fn completion(&self) -> enzui_widgets::gauge::Sampled<f32> {
        use enzui_widgets::gauge::Sampled;
        if self.finished {
            return Sampled::Sampled(1.0);
        }
        if self.sampled {
            Sampled::Sampled(self.progress)
        } else {
            Sampled::Unsampled
        }
    }
}

/// Fold one streamed progress update into the operation list.
///
/// Returns `true` when an operation was actually updated. Rules, each one a
/// behaviour round one had buried inside the poll-timer reducer arm:
///
/// * an update for an **unknown** attempt is dropped (no phantom row appears);
/// * an update for a **finished** operation is dropped (a late line from a
///   child that already exited must not un-finish a completed row);
/// * an **empty** message keeps the previous message rather than blanking it;
/// * progress is clamped to `0.0..=1.0`.
pub(crate) fn apply_progress(
    ops: &mut [OperationState],
    id: &OperationId,
    progress: f32,
    message: &str,
) -> bool {
    let Some(op) = ops
        .iter_mut()
        .find(|op| !op.finished && op.app_id == id.app_id && op.attempt == id.attempt)
    else {
        return false;
    };
    if progress.is_nan() {
        // A NaN line is not a measurement: it must neither move the bar nor
        // promote an unsampled operation to a determinate one.
    } else {
        op.progress = progress.clamp(0.0, 1.0);
        op.sampled = true;
    }
    if !message.is_empty() {
        op.message = message.to_owned();
    }
    true
}
