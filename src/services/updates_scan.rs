// Update badge, discovery, scan, and failure classification.

/// Partial and failed reads remain distinct from a confirmed empty update list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateBadge {
    /// A first check is still running. Not a claim, and deliberately silent:
    /// a boot-time spinner in the titlebar would be noise, and `Loading` is
    /// transient by construction.
    Checking,
    /// Every authorized scope answered and nothing is behind. The **only**
    /// state that licenses saying so.
    UpToDate,
    /// Every authorized scope answered; this many apps are behind.
    Behind(usize),
    /// At least one authorized scope did not answer. `known` is what the
    /// answering scopes reported, which may be zero — and zero here is not
    /// "up to date", it is "nothing found in the part I could see".
    Incomplete { known: usize },
}

impl UpdateBadge {}

/// Retain the previous update answer until the current scan generation settles; partial failures remain explicit.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Default)]
pub(crate) enum UpdateDiscovery {
    /// No check has completed yet. Not a claim about anything.
    #[default]
    Loading,
    /// Every authorized scope answered. `updates` may be empty, and **only
    /// here** does empty mean the user is current.
    Known {
        updates: Vec<AvailableUpdate>,
        /// When this answer was obtained, on the caller's activity clock.
        last_checked_ms: u64,
    },
    /// At least one authorized scope did not answer.
    ///
    /// Carries the partial list so a user is still told what *is* known — the
    /// alternative, discarding a scope's real findings because the other scope
    /// broke, replaces one silent lie with a louder one. Partial results are
    /// only ever rendered together with the unknown scopes, never as a
    /// complete answer.
    Failed {
        /// Ids from the scopes that answered. Never a complete list.
        partial: Vec<AvailableUpdate>,
        /// Every authorized scope, with its typed outcome.
        reports: Vec<UpdateScopeReport>,
        /// When the last *successful complete* check happened, if ever.
        last_checked_ms: Option<u64>,
        /// The generation whose scan produced this state. A Retry runs the
        /// next generation, so comparing this against the store's current
        /// generation is what tells the page a retry is in flight.
        retry_generation: u64,
    },
}

impl UpdateDiscovery {
    /// Classify a scan as Known only when every authorized scope answered.
    #[must_use]
    pub(crate) fn from_scan(
        scan: UpdateScan,
        generation: u64,
        now_ms: u64,
        previous: &Self,
    ) -> Self {
        if scan.all_scopes_completed() {
            return Self::Known {
                updates: scan.updates,
                last_checked_ms: now_ms,
            };
        }
        Self::Failed {
            partial: scan.updates,
            reports: scan.reports,
            // A failed check does not invalidate when the last *complete* one
            // happened; that age is what lets a user judge how stale the
            // partial picture is.
            last_checked_ms: previous.last_checked_ms(),
            retry_generation: generation,
        }
    }

    /// The actionable pending-update list in any state.
    ///
    /// Empty during `Loading` and possibly empty under `Failed` — which is
    /// why no caller may read emptiness as "up to date". Ask
    /// [`Self::claims_up_to_date`] instead.
    #[must_use]
    pub(crate) fn updates(&self) -> &[AvailableUpdate] {
        match self {
            Self::Loading => &[],
            Self::Known { updates, .. } => updates,
            Self::Failed { partial, .. } => partial,
        }
    }

    /// When the last complete check landed, if one ever did.
    #[must_use]
    pub(crate) fn last_checked_ms(&self) -> Option<u64> {
        match self {
            Self::Loading => None,
            Self::Known {
                last_checked_ms, ..
            } => Some(*last_checked_ms),
            Self::Failed {
                last_checked_ms, ..
            } => *last_checked_ms,
        }
    }

    /// The scopes whose answer is unknown, with their typed reasons.
    #[must_use]
    pub(crate) fn unknown_scopes(&self) -> Vec<&UpdateScopeReport> {
        match self {
            Self::Loading | Self::Known { .. } => Vec::new(),
            Self::Failed { reports, .. } => reports.iter().filter(|r| !r.completed()).collect(),
        }
    }

    /// Whether any surface should offer a Retry.
    ///
    /// Only when a failed scope is one the *user* owns retrying. A scan whose
    /// scopes all failed as `Cancelled` is already being replaced.
    #[must_use]
    pub(crate) fn offers_retry(&self) -> bool {
        self.unknown_scopes()
            .iter()
            .filter_map(|report| report.failure())
            .any(|failure| failure.retry_owner().offers_retry())
    }

    /// What the titlebar and nav may show.
    #[must_use]
    pub(crate) fn badge(&self) -> UpdateBadge {
        match self {
            Self::Loading => UpdateBadge::Checking,
            Self::Known { updates, .. } if updates.is_empty() => UpdateBadge::UpToDate,
            Self::Known { updates, .. } => UpdateBadge::Behind(updates.len()),
            Self::Failed { partial, .. } => UpdateBadge::Incomplete {
                known: partial.len(),
            },
        }
    }

    /// Settle only the completed operation's installation, retaining other scopes and partial failures.
    pub(crate) fn settle_operation(&mut self, app_id: &str, scope: &InstallScope) {
        let list = match self {
            Self::Loading => return,
            Self::Known { updates, .. } => updates,
            Self::Failed { partial, .. } => partial,
        };
        list.retain_mut(|update| update.app_id != app_id || update.settle(scope));
    }
}

// Bound scope failure reasons so two notices and Retry fit above Library content.
pub(crate) const UPDATE_FAILURE_CHAR_CAP: usize = 220;

/// Keep installation-specific failures typed so retry policy can distinguish their causes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum UpdateScanFailure {
    /// The query exceeded its deadline. The commonest real cause of the
    /// reported defect: a network stall on `remote-ls` looks exactly like
    /// "nothing is out of date".
    ///
    /// `after_ms` is zero when the limit is unknown; `remote` names the one
    /// repository that did not answer, when the scope was asked per remote.
    Timeout {
        after_ms: u64,
        remote: Option<String>,
    },
    /// A newer read replaced this one, so this scope's answer is not the
    /// current answer.
    Cancelled,
    /// `flatpak` ran and reported a failure. Carries the backend diagnostic.
    Failed(String),
    /// The query was never issued — no `flatpak` to run, or an unusable
    /// installation option.
    NotQueried(String),
}

impl UpdateScanFailure {
    /// The bounded, sanitized sentence shown to the user.
    ///
    /// Sanitized because `Failed` and `NotQueried` carry backend text, and
    /// capped because two of these render together.
    #[must_use]
    pub(crate) fn disclosure(&self) -> String {
        let sentence = match self {
            Self::Timeout { after_ms, remote } => {
                let within = if *after_ms == 0 {
                    String::new()
                } else {
                    format!(" within {}", format_duration(*after_ms))
                };
                match remote {
                    Some(remote) => {
                        format!("the {remote} repository did not answer{within}.")
                    }
                    None => format!("the check did not finish{within}."),
                }
            }
            Self::Cancelled => "the check was replaced by a newer one.".to_owned(),
            Self::Failed(diagnostic) => format!("flatpak reported: {diagnostic}"),
            Self::NotQueried(reason) => format!("the check could not run: {reason}"),
        };
        crate::services::flatpak_failure::cap_chars(
            &crate::services::flatpak_failure::sanitize_diagnostic(&sentence),
            UPDATE_FAILURE_CHAR_CAP,
        )
    }

    /// Superseded requests belong to the newer scan and must not offer a competing user retry.
    #[must_use]
    pub(crate) fn retry_owner(&self) -> RetryOwner {
        match self {
            Self::Timeout { .. } | Self::Failed(_) => RetryOwner::User,
            Self::Cancelled => RetryOwner::PendingLoad,
            Self::NotQueried(_) => RetryOwner::Unretryable,
        }
    }
}

fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        return format!("{ms} ms");
    }
    let seconds = ms / 1_000;
    if seconds == 1 {
        "1 second".to_owned()
    } else {
        format!("{seconds} seconds")
    }
}

/// One report per authorized scope, including empty answers and failures.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct UpdateScopeReport {
    /// The installation queried.
    pub scope: InstallScope,
    /// `Ok` with the number of app ids this scope reported, or why it did not
    /// answer.
    ///
    /// The count is kept for disclosure ("the user installation reported 3")
    /// and is not the actionable list: the merged [`UpdateScan::updates`] is,
    /// and each entry there carries its own reporting scopes.
    pub outcome: Result<usize, UpdateScanFailure>,
}

impl UpdateScopeReport {
    /// Whether this scope answered.
    #[must_use]
    pub(crate) fn completed(&self) -> bool {
        self.outcome.is_ok()
    }

    /// The failure, when this scope did not answer.
    #[must_use]
    pub(crate) fn failure(&self) -> Option<&UpdateScanFailure> {
        self.outcome.as_ref().err()
    }
}

/// Merge answered scopes while retaining every scope report, including failures.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct UpdateScan {
    /// One entry per app id, merged across the scopes that answered.
    pub updates: Vec<AvailableUpdate>,
    /// Every authorized scope, answered or not.
    pub reports: Vec<UpdateScopeReport>,
}

impl UpdateScan {
    pub(crate) fn failed(failure: UpdateScanFailure) -> Self {
        Self {
            updates: Vec::new(),
            reports: UPDATE_READ_SCOPES
                .iter()
                .map(|scope| UpdateScopeReport {
                    scope: scope.installation(),
                    outcome: Err(failure.clone()),
                })
                .collect(),
        }
    }

    // Derive successful fixture tallies from each entry's scopes rather than an independent count.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn complete(updates: Vec<AvailableUpdate>) -> Self {
        let reports = UPDATE_READ_SCOPES
            .iter()
            .map(|scope| {
                let installation = scope.installation();
                let reported = updates
                    .iter()
                    .filter(|update| update.reported_in(&installation))
                    .count();
                UpdateScopeReport {
                    scope: installation,
                    outcome: Ok(reported),
                }
            })
            .collect();
        Self { updates, reports }
    }

    /// Whether every authorized scope answered.
    ///
    /// The **only** predicate that licenses an "up to date" claim, and only
    /// then in combination with an empty `updates`.
    #[must_use]
    pub(crate) fn all_scopes_completed(&self) -> bool {
        !self.reports.is_empty() && self.reports.iter().all(UpdateScopeReport::completed)
    }

    /// The scopes that did not answer.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn failed_reports(&self) -> Vec<&UpdateScopeReport> {
        self.reports.iter().filter(|r| !r.completed()).collect()
    }

    /// The scopes that did answer.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn completed_scopes(&self) -> Vec<InstallScope> {
        self.reports
            .iter()
            .filter(|r| r.completed())
            .map(|r| r.scope.clone())
            .collect()
    }
}

// `flatpak remote-ls --updates`, once per installation scope. Returns app ids
// only.
pub(crate) fn list_updates_cancellable(cancellation: OperationCancellation) -> UpdateScan {
    let executor = ProcessFlatpakExecutor::cancellable(cancellation.clone());
    list_updates_with_executor(fixture_enabled(), &executor, &cancellation)
}

/// Update-list read with the fixture flag supplied directly.
pub(crate) fn list_updates_with_fixture(fixture: bool) -> UpdateScan {
    list_updates_with_executor(
        fixture,
        &ProcessFlatpakExecutor::default(),
        &OperationCancellation::default(),
    )
}

// Every queried installation contributes a report, including failed reads.
pub(crate) fn list_updates_with_executor(
    fixture: bool,
    executor: &dyn FlatpakExecutor,
    cancellation: &OperationCancellation,
) -> UpdateScan {
    // Fixture scans answer every scope without reaching host Flatpak.
    if fixture {
        let updates = fixture_updates();
        let reports = UPDATE_READ_SCOPES
            .iter()
            .map(|scope| UpdateScopeReport {
                scope: scope.installation(),
                outcome: Ok(updates
                    .iter()
                    .filter(|update| update.reported_in(&scope.installation()))
                    .count()),
            })
            .collect();
        return UpdateScan { updates, reports };
    }
    let mut updates: Vec<AvailableUpdate> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut reports: Vec<UpdateScopeReport> = Vec::new();
    for scope in UPDATE_READ_SCOPES {
        // A scope whose read is already moot still gets a report, so the
        // "both scopes settle" invariant holds on the cancellation path too.
        if cancellation.is_cancelled() {
            reports.push(UpdateScopeReport {
                scope: scope.installation(),
                outcome: Err(UpdateScanFailure::Cancelled),
            });
            continue;
        }
        if scope.may_refresh_appstream {
            // Best-effort: stale metadata makes the read below less accurate,
            // it does not make it fail.
            let _ = run_flatpak(executor, &["update", "--appstream", scope.option]);
        }
        let (raw, failure) = scope_update_listing(executor, scope.option);
        match raw {
            Some(raw) => {
                let installation = scope.installation();
                let mut reported = 0usize;
                for id in parse_columns(&raw)
                    .into_iter()
                    .filter_map(|cols| cols.into_iter().next())
                    .filter(|id| crate::flatpak_args::validate_app_id(id).is_ok())
                {
                    // One ID retains every reporting scope so a system-only update cannot become a user mutation.
                    reported += 1;
                    match index.get(&id) {
                        Some(&at) => updates[at].also_reported_in(installation.clone()),
                        None => {
                            index.insert(id.clone(), updates.len());
                            updates.push(AvailableUpdate::in_scope(id, installation.clone()));
                        }
                    }
                }
                // A remote that did not answer leaves the scope incomplete;
                // what the others reported is still real.
                reports.push(UpdateScopeReport {
                    scope: installation,
                    outcome: failure
                        .as_ref()
                        .map_or(Ok(reported), |failure| Err(failure.classify())),
                });
            }
            None => reports.push(UpdateScopeReport {
                scope: scope.installation(),
                outcome: Err(failure.as_ref().map_or_else(
                    || classify_update_failure(None, ""),
                    ScopeFailure::classify,
                )),
            }),
        }
    }
    // One scope answering is a real, if partial, update list, and reporting it
    // beats reporting nothing — but it is reported ALONGSIDE the other scope's
    // failure, never in place of it. Deciding what that combination licenses
    // is the caller's job, and `UpdateScan` gives it the facts to decide with:
    // a scope that failed reported nothing, so no id carries its installation
    // and nothing can be retargeted at it. A partial read narrows what is
    // actionable; it never widens it.
    UpdateScan { updates, reports }
}

// One scope's update listing, asked of each enabled remote on its own so an
// unreachable remote (Flathub offline, a mirror down) does not hide the
// updates the others report. Returns the combined listing, if any remote
// answered, and the first failure, if any did not. When the remotes cannot be
// read, the scope is asked as a whole.
fn scope_update_listing(
    executor: &dyn FlatpakExecutor,
    option: &str,
) -> (Option<String>, Option<ScopeFailure>) {
    let whole = |executor: &dyn FlatpakExecutor| match run_flatpak(
        executor,
        &["remote-ls", "--updates", "--columns=application", option],
    ) {
        Ok(raw) => (Some(raw), None),
        Err(error) => (
            None,
            Some(ScopeFailure {
                remote: None,
                error,
            }),
        ),
    };
    let Ok(raw) = run_flatpak(executor, &["remotes", "--columns=name,options", option]) else {
        return whole(executor);
    };
    let remotes: Vec<String> = parse_columns(&raw)
        .into_iter()
        .filter(|cols| {
            !cols
                .get(1)
                .is_some_and(|options| options.split(',').any(|o| o.trim() == "disabled"))
        })
        .filter_map(|cols| cols.into_iter().next())
        .filter(|name| crate::flatpak_args::validate_remote_name(name).is_ok())
        .collect();
    if remotes.is_empty() {
        return whole(executor);
    }
    let mut listing: Option<String> = None;
    let mut failure = None;
    for remote in &remotes {
        match run_flatpak(
            executor,
            &[
                "remote-ls",
                "--updates",
                "--columns=application",
                option,
                remote,
            ],
        ) {
            Ok(raw) => listing.get_or_insert_with(String::new).push_str(&raw),
            Err(error) => {
                failure.get_or_insert(ScopeFailure {
                    remote: Some(remote.clone()),
                    error,
                });
            }
        }
        if let Some(text) = listing
            .as_mut()
            .filter(|text| !text.ends_with('\n') && !text.is_empty())
        {
            text.push('\n');
        }
    }
    (listing, failure)
}

// Timeout classification uses the executor-owned failure format.
fn classify_update_failure(remote: Option<&str>, error: &str) -> UpdateScanFailure {
    // Localized numbers arrive wrapped in Unicode isolation marks.
    let lower = error
        .replace(['\u{2068}', '\u{2069}'], "")
        .to_ascii_lowercase();
    let remote = remote.map(str::to_owned);
    if let Some(ms) = parse_timeout_ms(&lower) {
        return UpdateScanFailure::Timeout {
            after_ms: ms,
            remote,
        };
    }
    if lower.contains("timed out") || lower.contains("deadline") {
        return UpdateScanFailure::Timeout {
            after_ms: 0,
            remote,
        };
    }
    // An absent or unusable program never ran, so there is nothing to retry
    // until the environment changes.
    if lower.contains("no such file or directory")
        || lower.contains("not found")
        || lower.contains("permission denied")
    {
        return UpdateScanFailure::NotQueried(error.to_owned());
    }
    UpdateScanFailure::Failed(error.to_owned())
}

/// One scope's first failure, and the repository it came from when the
/// scope was asked one remote at a time.
#[derive(Clone, Debug)]
struct ScopeFailure {
    remote: Option<String>,
    error: String,
}

impl ScopeFailure {
    fn classify(&self) -> UpdateScanFailure {
        classify_update_failure(self.remote.as_deref(), &self.error)
    }
}

// Recover the millisecond budget from a timeout diagnostic, when present:
// the executor's own "exceeded its N ms deadline", or "timed out after N".
fn parse_timeout_ms(lower: &str) -> Option<u64> {
    let at = ["exceeded its ", "timed out after "]
        .iter()
        .find_map(|marker| lower.find(marker).map(|at| at + marker.len()))?;
    let digits: String = lower[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    let value: u64 = digits.parse().ok()?;
    if lower[at + digits.len()..].trim_start().starts_with("ms") {
        Some(value)
    } else {
        Some(value.saturating_mul(1_000))
    }
}
