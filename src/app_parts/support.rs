//! Store support types: reads, drains, owners, operations.

use super::*;

// The typed outcome of the detail page's background permission-metadata read.
//
// A read failure is a distinct, retryable state, never an authoritative empty
// permission set. An empty [`services::Permissions`] means the metadata read
// succeeded and the app declares no sandbox exceptions; a transport or process
// failure is [`PermissionRead::Unavailable`] and must never be rendered as
// that reassuring "no exceptions" claim.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum PermissionRead {
    /// The background read is in flight; no result has landed yet.
    #[default]
    Loading,
    /// The metadata read succeeded. The declared set may be legitimately empty.
    Loaded(services::Permissions),
    /// The read failed (no flatpak, app absent, or a transport/process
    /// refusal). Retryable, and never a declared-nothing permission set.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseDrainOutcome {
    Joining { workers: usize },
    Joined,
    Panicked { workers: usize },
    TimedOut { workers: usize },
    StartFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CloseDrainReport {
    pub(crate) catalog: CloseDrainOutcome,
    pub(crate) detail: CloseDrainOutcome,
    pub(crate) reads: CloseDrainOutcome,
    pub(crate) mutations: CloseDrainOutcome,
    pub(crate) exports: CloseDrainOutcome,
}

impl CloseDrainReport {
    pub(crate) fn outcomes(self) -> [CloseDrainOutcome; 5] {
        [
            self.catalog,
            self.detail,
            self.reads,
            self.mutations,
            self.exports,
        ]
    }

    pub(crate) fn completed(self) -> bool {
        self.outcomes()
            .into_iter()
            .all(|outcome| matches!(outcome, CloseDrainOutcome::Joined))
    }

    pub(crate) fn start_failed(self) -> bool {
        self.outcomes()
            .into_iter()
            .any(|outcome| matches!(outcome, CloseDrainOutcome::StartFailed))
    }

    pub(crate) fn has_panicked(self) -> bool {
        self.outcomes()
            .into_iter()
            .any(|outcome| matches!(outcome, CloseDrainOutcome::Panicked { .. }))
    }

    pub(crate) fn has_timed_out(self) -> bool {
        self.outcomes()
            .into_iter()
            .any(|outcome| matches!(outcome, CloseDrainOutcome::TimedOut { .. }))
    }

    pub(crate) fn still_joining(self) -> bool {
        self.outcomes().into_iter().any(|outcome| {
            matches!(
                outcome,
                CloseDrainOutcome::Joining { .. } | CloseDrainOutcome::TimedOut { .. }
            )
        })
    }

    pub(crate) fn display_after(self, deadline_expired: bool) -> Self {
        if !deadline_expired {
            return self;
        }
        let timed_out = |outcome| match outcome {
            CloseDrainOutcome::Joining { workers } => CloseDrainOutcome::TimedOut { workers },
            settled => settled,
        };
        Self {
            catalog: timed_out(self.catalog),
            detail: timed_out(self.detail),
            reads: timed_out(self.reads),
            mutations: timed_out(self.mutations),
            exports: timed_out(self.exports),
        }
    }

    pub(crate) fn affected_workers(self) -> usize {
        self.outcomes()
            .into_iter()
            .map(|outcome| match outcome {
                CloseDrainOutcome::Joining { workers }
                | CloseDrainOutcome::Panicked { workers }
                | CloseDrainOutcome::TimedOut { workers } => workers,
                CloseDrainOutcome::Joined | CloseDrainOutcome::StartFailed => 0,
            })
            .sum()
    }
}

pub(crate) struct StoreExportOwner {
    owner: enzui_core::blocking_task::BlockingTaskOwner,
}

impl StoreExportOwner {
    pub(crate) fn start() -> std::io::Result<Self> {
        enzui_core::blocking_task::BlockingTaskOwner::new(
            "store-portal-export",
            enzui_widgets::export_pool::EXPORT_WORKER_COUNT,
            enzui_widgets::export_pool::EXPORT_QUEUE_CAPACITY,
        )
        .map(|owner| Self { owner })
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            owner: enzui_core::blocking_task::BlockingTaskOwner::unavailable(
                enzui_widgets::export_pool::EXPORT_QUEUE_CAPACITY,
            ),
        }
    }

    pub(crate) fn dispatcher(&self) -> enzui_core::blocking_task::BlockingTaskDispatcher {
        self.owner.dispatcher()
    }

    pub(crate) fn shutdown(&mut self) {
        self.owner.shutdown();
    }

    pub(crate) fn shutdown_and_join(self) -> enzui_core::blocking_task::BlockingShutdown {
        self.owner.shutdown_and_join()
    }
}

pub(crate) struct CloseOwners {
    pub(crate) catalog: enzui_core::blocking_task::BlockingTaskOwner,
    pub(crate) detail: enzui_core::blocking_task::BlockingTaskOwner,
    pub(crate) reads: enzui_core::blocking_task::BlockingTaskOwner,
    pub(crate) mutations: enzui_core::blocking_task::BlockingTaskOwner,
    pub(crate) exports: StoreExportOwner,
}

#[derive(Clone, Debug)]
pub(crate) struct CloseDrainProbe {
    pub(crate) catalog: enzui_core::blocking_task::BlockingTaskDispatcher,
    pub(crate) detail: enzui_core::blocking_task::BlockingTaskDispatcher,
    pub(crate) reads: enzui_core::blocking_task::BlockingTaskDispatcher,
    pub(crate) mutations: enzui_core::blocking_task::BlockingTaskDispatcher,
    pub(crate) exports: enzui_core::blocking_task::BlockingTaskDispatcher,
}

impl CloseDrainProbe {
    pub(crate) fn live(&self) -> usize {
        self.catalog
            .snapshot()
            .live()
            .saturating_add(self.detail.snapshot().live())
            .saturating_add(self.reads.snapshot().live())
            .saturating_add(self.mutations.snapshot().live())
            .saturating_add(self.exports.snapshot().live())
    }

    pub(crate) fn joining_report(&self) -> CloseDrainReport {
        CloseDrainReport {
            catalog: CloseDrainOutcome::Joining {
                workers: self.catalog.snapshot().worker_capacity,
            },
            detail: CloseDrainOutcome::Joining {
                workers: self.detail.snapshot().worker_capacity,
            },
            reads: CloseDrainOutcome::Joining {
                workers: self.reads.snapshot().worker_capacity,
            },
            mutations: CloseDrainOutcome::Joining {
                workers: self.mutations.snapshot().worker_capacity,
            },
            exports: CloseDrainOutcome::Joining {
                workers: self.exports.snapshot().worker_capacity,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseHelperStatus {
    Running,
    ReaperDebt,
    Joined,
    Panicked,
    NotStarted,
}

pub(crate) struct CloseHelperDebt {
    id: u64,
    handle: std::thread::JoinHandle<()>,
}

pub(crate) const CLOSE_HELPER_DEBT_RESERVE: usize = 4;
static NEXT_CLOSE_HELPER_DEBT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static CLOSE_HELPER_DEBT: std::sync::OnceLock<std::sync::Mutex<Vec<CloseHelperDebt>>> =
    std::sync::OnceLock::new();

pub(crate) fn close_helper_debt() -> &'static std::sync::Mutex<Vec<CloseHelperDebt>> {
    CLOSE_HELPER_DEBT
        .get_or_init(|| std::sync::Mutex::new(Vec::with_capacity(CLOSE_HELPER_DEBT_RESERVE)))
}

pub(crate) fn retain_close_helper(handle: std::thread::JoinHandle<()>) -> u64 {
    let id = NEXT_CLOSE_HELPER_DEBT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    close_helper_debt()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(CloseHelperDebt { id, handle });
    id
}

pub(crate) fn poll_retained_close_helper(id: u64) -> Option<CloseHelperStatus> {
    let mut debt = close_helper_debt()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let index = debt.iter().position(|entry| entry.id == id)?;
    if !debt[index].handle.is_finished() {
        return None;
    }
    let entry = debt.swap_remove(index);
    Some(if entry.handle.join().is_ok() {
        CloseHelperStatus::Joined
    } else {
        CloseHelperStatus::Panicked
    })
}

pub(crate) struct CloseDrainSession {
    pub(crate) report: std::sync::Arc<std::sync::Mutex<CloseDrainReport>>,
    pub(crate) helper: Option<std::thread::JoinHandle<()>>,
    pub(crate) helper_debt: Option<u64>,
    pub(crate) helper_status: CloseHelperStatus,
}

impl CloseDrainSession {
    pub(crate) fn poll(
        &mut self,
        deadline_expired: bool,
        helper_overtime: bool,
    ) -> (CloseDrainReport, CloseHelperStatus) {
        if let Some(handle) = self.helper.as_ref()
            && handle.is_finished()
        {
            let handle = self
                .helper
                .take()
                .expect("finished close helper is retained");
            self.helper_status = if handle.join().is_ok() {
                CloseHelperStatus::Joined
            } else {
                CloseHelperStatus::Panicked
            };
        } else if helper_overtime && let Some(handle) = self.helper.take() {
            self.helper_debt = Some(retain_close_helper(handle));
            self.helper_status = CloseHelperStatus::ReaperDebt;
        }
        if let Some(id) = self.helper_debt
            && let Some(status) = poll_retained_close_helper(id)
        {
            self.helper_debt = None;
            self.helper_status = status;
        }
        let mut report = self
            .report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.helper_status == CloseHelperStatus::Panicked {
            mark_close_outcome_panicked(&mut report.catalog);
            mark_close_outcome_panicked(&mut report.detail);
            mark_close_outcome_panicked(&mut report.reads);
            mark_close_outcome_panicked(&mut report.mutations);
            mark_close_outcome_panicked(&mut report.exports);
        }
        (report.display_after(deadline_expired), self.helper_status)
    }
}

impl Drop for CloseDrainSession {
    fn drop(&mut self) {
        if let Some(handle) = self.helper.take() {
            self.helper_debt = Some(retain_close_helper(handle));
            self.helper_status = CloseHelperStatus::ReaperDebt;
        }
    }
}

pub(crate) fn close_is_authorized(report: CloseDrainReport, helper: CloseHelperStatus) -> bool {
    report.completed() && helper == CloseHelperStatus::Joined
}

pub(crate) fn mark_close_outcome_panicked(outcome: &mut CloseDrainOutcome) {
    if let CloseDrainOutcome::Joining { workers } = *outcome {
        *outcome = CloseDrainOutcome::Panicked { workers };
    }
}

// The typed state of the Sandbox Permissions editable model.
//
// A build failure is [`PermissionTogglesState::Unavailable`], distinct from a
// load still in flight and never an all-disabled editable set, so the reducer
// keeps mutation controls disabled until the model is `Ready`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum PermissionTogglesState {
    /// The build is in flight; no outcome has landed yet.
    #[default]
    Loading,
    /// An authoritative model built from clean base-metadata and override
    /// reads.
    Ready(Vec<services::PermissionToggle>),
    /// Base metadata or override state was unknown, so no editable model
    /// exists. Retryable, and never an all-disabled set.
    Unavailable,
}

// Production waits for its repository read; test stores use declared repositories unless overridden explicitly.
pub(crate) fn default_remotes_load() -> services::RemoteLoad {
    #[cfg(test)]
    {
        services::RemoteLoad::Loaded
    }
    #[cfg(not(test))]
    {
        services::RemoteLoad::Loading
    }
}

// The repositories a freshly constructed [`Store`] starts with. Empty in
// production; the declared fixture set under `cfg(test)`. See
// [`default_remotes_load`].
pub(crate) fn default_remotes() -> Vec<RemoteInfo> {
    #[cfg(test)]
    {
        services::fixture_remotes()
    }
    #[cfg(not(test))]
    {
        Vec::new()
    }
}

/// Shared progress label while cancellation reaps the operation process group.
pub(crate) const CANCELLING_MESSAGE: &str = "Cancelling…";
pub(crate) const DETAIL_GENERATION_EXHAUSTED: &str =
    "Details cannot be refreshed again in this session. Restart Store to continue.";
pub(crate) const CLOSE_RECOVERY_UNKNOWN: &str = "Background requests were stopped while closing. Their results are unknown; refresh before relying on this page.";
pub(crate) const CLOSE_RECOVERY_PANICKED: &str = "A background worker ended unexpectedly while closing. Its result is unknown; refresh before relying on this page.";

pub(crate) struct OperationCompletion {
    pub(crate) task:
        std::pin::Pin<Box<enzui_core::blocking_task::BlockingTask<flatpak::OperationTerminal>>>,
    pub(crate) progress_drained: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) worker_exited: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl OperationCompletion {
    pub(crate) fn poll(&mut self) -> Option<flatpak::OperationTerminal> {
        if !self
            .progress_drained
            .load(std::sync::atomic::Ordering::Acquire)
            || !self
                .worker_exited
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return None;
        }
        let waker = iced::futures::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        match std::future::Future::poll(self.task.as_mut(), &mut context) {
            std::task::Poll::Ready(settlement) => {
                Some(flatpak::operation_terminal(settlement.into_result()))
            }
            std::task::Poll::Pending => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnedReadDomain {
    Installed,
    Remotes,
    Updates,
    PermissionToggles,
}

#[derive(Clone, Debug)]
pub(crate) enum OwnedReadIntent {
    Installed {
        request: u64,
    },
    Remotes {
        request: u64,
    },
    Updates {
        request: u64,
        generation: u64,
    },
    PermissionToggles {
        request: u64,
        app_id: String,
        generation: u64,
    },
}

impl OwnedReadIntent {
    pub(crate) fn request(&self) -> u64 {
        match self {
            Self::Installed { request }
            | Self::Remotes { request }
            | Self::Updates { request, .. }
            | Self::PermissionToggles { request, .. } => *request,
        }
    }

    pub(crate) fn domain(&self) -> OwnedReadDomain {
        match self {
            Self::Installed { .. } => OwnedReadDomain::Installed,
            Self::Remotes { .. } => OwnedReadDomain::Remotes,
            Self::Updates { .. } => OwnedReadDomain::Updates,
            Self::PermissionToggles { .. } => OwnedReadDomain::PermissionToggles,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct DeferredOwnedReads {
    pub(crate) installed: Option<OwnedReadIntent>,
    pub(crate) remotes: Option<OwnedReadIntent>,
    updates: Option<OwnedReadIntent>,
    permission_toggles: Option<OwnedReadIntent>,
    next_domain: usize,
    pub(crate) retry_in_flight: Option<u64>,
}

impl DeferredOwnedReads {
    pub(crate) const DOMAIN_COUNT: usize = 4;

    pub(crate) fn slot_mut(&mut self, domain: OwnedReadDomain) -> &mut Option<OwnedReadIntent> {
        match domain {
            OwnedReadDomain::Installed => &mut self.installed,
            OwnedReadDomain::Remotes => &mut self.remotes,
            OwnedReadDomain::Updates => &mut self.updates,
            OwnedReadDomain::PermissionToggles => &mut self.permission_toggles,
        }
    }

    pub(crate) fn retain_latest(&mut self, intent: OwnedReadIntent) {
        let slot = self.slot_mut(intent.domain());
        if slot
            .as_ref()
            .is_none_or(|pending| pending.request() < intent.request())
        {
            *slot = Some(intent);
        }
    }

    pub(crate) fn take_next(&mut self) -> Option<OwnedReadIntent> {
        for offset in 0..Self::DOMAIN_COUNT {
            let index = (self.next_domain + offset) % Self::DOMAIN_COUNT;
            let domain = match index {
                0 => OwnedReadDomain::Installed,
                1 => OwnedReadDomain::Remotes,
                2 => OwnedReadDomain::Updates,
                _ => OwnedReadDomain::PermissionToggles,
            };
            if let Some(intent) = self.slot_mut(domain).take() {
                self.next_domain = (index + 1) % Self::DOMAIN_COUNT;
                return Some(intent);
            }
        }
        None
    }

    pub(crate) fn retry_started(&mut self, request: u64) {
        self.retry_in_flight = Some(request);
    }

    pub(crate) fn retry_settled(&mut self, request: u64) {
        if self.retry_in_flight == Some(request) {
            self.retry_in_flight = None;
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.installed.is_none()
            && self.remotes.is_none()
            && self.updates.is_none()
            && self.permission_toggles.is_none()
    }

    pub(crate) fn needs_wake(&self) -> bool {
        !self.is_empty() && self.retry_in_flight.is_none()
    }
}

// The read pool's fixed worker count: two blocking reads run at once.
pub(crate) const READ_WORKER_COUNT: usize = 2;
// The read pool's fixed queue depth beyond the active set. The aggregate
// admission ceiling is [`READ_WORKER_COUNT`] active plus this many queued.
pub(crate) const READ_QUEUE_CAPACITY: usize = 4;
// Catalogue cache publication is last-writer-wins, so one FIFO worker keeps
// disk publication in dispatch-generation order.
pub(crate) const CATALOG_WORKER_COUNT: usize = 1;
pub(crate) const CATALOG_QUEUE_CAPACITY: usize = 4;
pub(crate) const DETAIL_WORKER_COUNT: usize = 3;
pub(crate) const DETAIL_QUEUE_CAPACITY: usize = 6;
pub(crate) const OPERATION_CONCURRENCY_LIMIT: usize = 4;
pub(crate) const OPERATION_QUEUE_CAPACITY: usize = OPERATION_CONCURRENCY_LIMIT;
pub(crate) const OPERATION_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(50);
pub(crate) const OPERATION_BUSY: &str =
    "Store is busy with other package changes. Try again when one finishes.";
pub(crate) const OPERATION_ATTEMPT_EXHAUSTED: &str =
    "Store cannot start another package change for this app in this session.";
pub(crate) const OPERATION_ACTIVITY_EXHAUSTED: &str =
    "Store cannot track another package change in this session.";

pub(crate) fn blocking_owner(
    name: &str,
    workers: usize,
    queue: usize,
) -> enzui_core::blocking_task::BlockingTaskOwner {
    let workers = std::num::NonZeroUsize::new(workers).expect("worker count is nonzero");
    let queue = std::num::NonZeroUsize::new(queue).expect("queue capacity is nonzero");
    enzui_core::blocking_task::BlockingTaskOwner::new(name, workers, queue)
        .unwrap_or_else(|_| enzui_core::blocking_task::BlockingTaskOwner::unavailable(queue))
}

// Payload-free, retryable message shown when the read pool declines a remote
// mutation before it runs.
pub(crate) const READ_POOL_REMOTE_REFUSAL: &str =
    "The system was busy and the change was not attempted; try again.";

// Placeholder reason when the read pool's owner is gone (shutdown or a
// panicked worker); a Busy saturation is retained and retried instead.
pub(crate) const FEATURED_IMAGE_UNAVAILABLE: &str = "screenshot service unavailable";

pub(crate) fn read_terminal_text(subject: &str, terminal: flatpak::ReadTaskError) -> String {
    format!("{subject}: {}", terminal.message())
}

// Cadence of pending-only bounded-pool retry wakes.
pub(crate) const FEATURED_RETRY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(300);
pub(crate) const READ_OWNER_RETRY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);

// Map a bounded read-pool submission to a message: a completed read to
// `on_ready`, a refusal (saturation or shutdown) to `on_refused`. The future
// awaits only the settlement, so a refusal does not block the runtime.
pub(crate) fn read_pool_task<T>(
    task: enzui_core::blocking_task::BlockingTask<T>,
    on_ready: impl FnOnce(T) -> Message + Send + 'static,
    on_refused: impl FnOnce(enzui_core::blocking_task::BlockingTaskError) -> Message + Send + 'static,
) -> Task<Message>
where
    T: Send + 'static,
{
    Task::future(async move {
        match task.await.into_result() {
            Ok(value) => on_ready(value),
            Err(error) => on_refused(error),
        }
    })
}

pub(crate) fn store_export_parent_task(
    pool: &enzui_core::blocking_task::BlockingTaskDispatcher,
    generation: u64,
    window: iced::window::Id,
    cancel: enzui_core::portal::cancel::PortalCancel,
    timeout: std::time::Duration,
) -> Task<
    enzui_parent_lease::Handoff<
        enzui_core::blocking_task::BlockingTaskSettlement<enzui_wayland::ParentAdoption>,
    >,
> {
    use enzui_core::blocking_task::{BlockingTaskError, BlockingTaskSettlement};
    use enzui_parent_lease::Handoff;

    let pool = pool.clone();
    iced::window::wayland_parent_export(window)
        .map(Handoff::new)
        .then(move |carrier| {
            let carrier = carrier.clone();
            let cancel = cancel.clone();
            let pool = pool.clone();
            Task::future(async move {
                if cancel.is_terminal() {
                    return Handoff::new(BlockingTaskSettlement::from_error(
                        generation,
                        BlockingTaskError::Cancelled,
                    ));
                }
                let payload = carrier.take().flatten();
                let cancel_for_work = cancel.clone();
                let task = pool.submit(generation, move |_ctx| {
                    enzui_wayland::adopt_parent(payload, &cancel_for_work, timeout)
                });
                Handoff::new(task.await)
            })
        })
}

pub(crate) const CLOSE_OWNER_WAIT: std::time::Duration = std::time::Duration::from_millis(250);
pub(crate) const CLOSE_HELPER_CUSTODY_LIMIT: std::time::Duration =
    std::time::Duration::from_secs(21);

pub(crate) fn close_drain_outcome(
    outcome: enzui_core::blocking_task::BlockingShutdown,
) -> CloseDrainOutcome {
    match outcome {
        enzui_core::blocking_task::BlockingShutdown::Joined { panicked: 0, .. } => {
            CloseDrainOutcome::Joined
        }
        enzui_core::blocking_task::BlockingShutdown::Joined { panicked, .. } => {
            CloseDrainOutcome::Panicked { workers: panicked }
        }
        _ => CloseDrainOutcome::Panicked { workers: 1 },
    }
}

impl CloseOwners {
    pub(crate) fn probe(&self) -> CloseDrainProbe {
        CloseDrainProbe {
            catalog: self.catalog.dispatcher(),
            detail: self.detail.dispatcher(),
            reads: self.reads.dispatcher(),
            mutations: self.mutations.dispatcher(),
            exports: self.exports.dispatcher(),
        }
    }

    fn shutdown_and_join(mut self, report: &std::sync::Mutex<CloseDrainReport>) {
        self.catalog.shutdown();
        self.detail.shutdown();
        self.reads.shutdown();
        self.mutations.shutdown();
        self.exports.shutdown();

        let catalog = close_drain_outcome(self.catalog.shutdown_and_join());
        report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .catalog = catalog;

        let detail = close_drain_outcome(self.detail.shutdown_and_join());
        report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .detail = detail;

        let reads = close_drain_outcome(self.reads.shutdown_and_join());
        report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reads = reads;

        let mutations = close_drain_outcome(self.mutations.shutdown_and_join());
        report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .mutations = mutations;

        let exports = close_drain_outcome(self.exports.shutdown_and_join());
        report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .exports = exports;
    }
}

pub(crate) fn close_drain_start_failed() -> CloseDrainReport {
    CloseDrainReport {
        catalog: CloseDrainOutcome::StartFailed,
        detail: CloseDrainOutcome::StartFailed,
        reads: CloseDrainOutcome::StartFailed,
        mutations: CloseDrainOutcome::StartFailed,
        exports: CloseDrainOutcome::StartFailed,
    }
}

#[allow(
    clippy::result_large_err,
    reason = "Admission refusal returns the exact resource owners without allocation"
)]
pub(crate) fn start_close_drain(owners: CloseOwners) -> Result<CloseDrainSession, CloseOwners> {
    start_close_drain_with(owners, |worker| {
        std::thread::Builder::new()
            .name("store-close-drain".to_owned())
            .spawn(worker)
    })
}

#[allow(
    clippy::result_large_err,
    reason = "Admission refusal returns the exact resource owners without allocation"
)]
pub(crate) fn start_close_drain_with(
    owners: CloseOwners,
    spawn_worker: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<std::thread::JoinHandle<()>>,
) -> Result<CloseDrainSession, CloseOwners> {
    let probe = owners.probe();
    let report = std::sync::Arc::new(std::sync::Mutex::new(probe.joining_report()));
    let worker_report = std::sync::Arc::clone(&report);
    let (owners_sender, owners_receiver) = std::sync::mpsc::sync_channel::<CloseOwners>(1);
    let worker: Box<dyn FnOnce() + Send> = Box::new(move || {
        let Ok(owners) = owners_receiver.recv() else {
            return;
        };
        let completed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            owners.shutdown_and_join(&worker_report);
        }));
        if completed.is_err() {
            let mut report = worker_report
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            mark_close_outcome_panicked(&mut report.catalog);
            mark_close_outcome_panicked(&mut report.detail);
            mark_close_outcome_panicked(&mut report.reads);
            mark_close_outcome_panicked(&mut report.mutations);
            mark_close_outcome_panicked(&mut report.exports);
        }
    });
    let helper = match spawn_worker(worker) {
        Ok(helper) => helper,
        Err(_) => return Err(owners),
    };
    if let Err(std::sync::mpsc::SendError(owners)) = owners_sender.send(owners) {
        retain_close_helper(helper);
        return Err(owners);
    }
    Ok(CloseDrainSession {
        report,
        helper: Some(helper),
        helper_debt: None,
        helper_status: CloseHelperStatus::Running,
    })
}

#[cfg(test)]
pub(crate) fn test_theme_integration(theme: EnzTheme) -> (ThemeIntegration, Task<ThemeEvent>) {
    ThemeIntegration::unmanaged_compiled_theme(theme)
}

// Preserve the previous policy value so rejected writes can undo optimistic state.
pub(crate) fn permission_override_finished(
    runner: &dyn services::PermissionOverrideRunner,
    app_id: &str,
    key: String,
    attempted: bool,
    previous: bool,
) -> Message {
    // Classify at the injected runner boundary: an invalid app ID never
    // reaches the runner, and a runner rejection's raw payload is diverted to
    // the escaped diagnostic sink so only authored copy is ever surfaced.
    let result = match flatpak_args::validate_app_id(app_id) {
        Err(_) => Err(services::PermissionActionFailure::invalid_app_id(
            services::PermissionAction::Write,
        )),
        Ok(()) => runner.apply(app_id, &key, attempted).map_err(|raw| {
            services::PermissionActionFailure::rejected(services::PermissionAction::Write, &raw)
        }),
    };
    Message::PermissionOverrideFinished {
        key,
        attempted,
        previous,
        result,
    }
}

// Execute a checked whole-policy reset through the same injected boundary.
pub(crate) fn permission_reset_finished(
    runner: &dyn services::PermissionOverrideRunner,
    app_id: &str,
    previous: Vec<services::PermissionToggle>,
) -> Message {
    let result = match flatpak_args::validate_app_id(app_id) {
        Err(_) => Err(services::PermissionActionFailure::invalid_app_id(
            services::PermissionAction::Reset,
        )),
        Ok(()) => runner.reset(app_id).map_err(|raw| {
            services::PermissionActionFailure::rejected(services::PermissionAction::Reset, &raw)
        }),
    };
    Message::PermissionResetFinished { previous, result }
}

// [`Store::operation_for`] against a borrowed operation list.
//
// Virtualised row builders capture `&store.operations` rather than the whole
// store; both entry points resolve an app's operation through this one rule.
pub(crate) fn operation_for_in<'a>(
    operations: &'a [model::OperationState],
    app_id: &str,
) -> Option<&'a model::OperationState> {
    operations.iter().rev().find(|op| op.app_id == app_id)
}
