//! The async boundary between blocking platform services and the iced runtime.
//!
//! Short Flatpak reads and mutations run on application-owned bounded pools.
//! Dropping an iced read task only requests cancellation; the pool keeps the
//! closure, child process, pipe readers, and exact worker join charged to its
//! owner through teardown.

use crate::model::{OpKind, OperationId};
use crate::services;
use enzui_core::blocking_task::{
    BlockingTask, BlockingTaskDispatcher, BlockingTaskError, BlockingTaskOwner,
};
use enzui_core::operation_activity::ActivityPaintThrottle;
use enzui_core::worker_bridge::{self, WorkerBridgeClose, WorkerReceiver};
use iced::futures::Stream;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

const OPERATION_LIFECYCLE_CAPACITY: usize = 1;
const OPERATION_RETAINED_CAPACITY: usize = OPERATION_LIFECYCLE_CAPACITY + 4;

/// A mutating flatpak operation the user explicitly asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Operation {
    pub id: OperationId,
    pub name: String,
    pub kind: OpKind,
    /// The parsed installation scope; unknown or unsupported scopes are refused before spawning a worker.
    pub scope: services::MutationScope,
    pub(crate) reviewed_install: Option<services::install_review::ReviewedInstall>,
}

/// The single terminal state of an admitted operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OperationTerminal {
    Succeeded,
    Failed(String),
    Cancelled,
}

/// One event from a running operation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OpEvent {
    Progress {
        id: OperationId,
        progress: f32,
        message: String,
    },
    Finished {
        id: OperationId,
        terminal: OperationTerminal,
    },
}

/// Why an application-owned read produced no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadTaskError {
    Busy,
    Cancelled,
    Shutdown,
    Panicked,
    Unknown,
}

impl ReadTaskError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Busy => {
                "Read capacity is busy; the latest request for this data will retry after another read finishes."
            }
            Self::Cancelled => "The read was cancelled before it completed.",
            Self::Shutdown => "The read service is shutting down.",
            Self::Panicked => "The read worker ended unexpectedly.",
            Self::Unknown => "The read service returned an unknown failure.",
        }
    }
}

impl From<BlockingTaskError> for ReadTaskError {
    fn from(error: BlockingTaskError) -> Self {
        match error {
            BlockingTaskError::Busy => Self::Busy,
            BlockingTaskError::Cancelled => Self::Cancelled,
            BlockingTaskError::Shutdown => Self::Shutdown,
            BlockingTaskError::Panicked => Self::Panicked,
            _ => Self::Unknown,
        }
    }
}

// Submit one process-backed read to an application-owned dispatcher.
//
// Dropping the returned iced task cancels its [`BlockingTask`] without joining
// on the UI thread. The owner retains admitted work until the closure exits.
pub(crate) fn owned_read_task<T, M>(
    dispatcher: BlockingTaskDispatcher,
    request: u64,
    cancellation: services::OperationCancellation,
    f: impl FnOnce(services::OperationCancellation) -> T + Send + 'static,
    on_ready: impl FnOnce(T) -> M + Send + 'static,
    on_terminal: impl FnOnce(ReadTaskError) -> M + Send + 'static,
) -> iced::Task<M>
where
    T: Send + 'static,
    M: Send + 'static,
{
    let task = dispatcher.submit(request, move |context| {
        f(cancellation.with_blocking_context(context))
    });
    iced::Task::future(async move {
        match task.await.into_result() {
            Ok(value) => on_ready(value),
            Err(error) => on_terminal(error.into()),
        }
    })
}

struct OperationStream {
    receiver: WorkerReceiver<OpEvent>,
    close: WorkerBridgeClose<OpEvent>,
    cancellation: services::OperationCancellation,
    progress_drained: Arc<AtomicBool>,
}

impl Stream for OperationStream {
    type Item = OpEvent;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let event = Pin::new(&mut self.receiver).poll_next(context);
        if matches!(&event, Poll::Ready(None)) {
            self.progress_drained.store(true, Ordering::Release);
        }
        event
    }
}

impl Drop for OperationStream {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.close.close();
        self.progress_drained.store(true, Ordering::Release);
    }
}

pub(crate) struct OperationTask {
    pub(crate) id: OperationId,
    pub(crate) progress: iced::Task<OpEvent>,
    pub(crate) completion: BlockingTask<OperationTerminal>,
    pub(crate) progress_drained: Arc<AtomicBool>,
    pub(crate) worker_exited: Arc<AtomicBool>,
}

struct OperationExit(Arc<AtomicBool>);

impl Drop for OperationExit {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn operation_stream(
    owner: &BlockingTaskOwner,
    op: Operation,
    program: PathBuf,
    cancellation: services::OperationCancellation,
    legacy_transport_fixture: bool,
) -> (
    OperationStream,
    BlockingTask<OperationTerminal>,
    Arc<AtomicBool>,
) {
    let capacity = NonZeroUsize::new(OPERATION_LIFECYCLE_CAPACITY)
        .expect("operation lifecycle capacity is nonzero");
    let (producer, receiver, close) = worker_bridge::channel(capacity);
    debug_assert_eq!(producer.retained_capacity(), OPERATION_RETAINED_CAPACITY);
    let worker_cancellation = cancellation.clone();
    let generation = op.id.attempt;
    let worker_exited = Arc::new(AtomicBool::new(false));
    let worker_exit = OperationExit(Arc::clone(&worker_exited));
    let completion = owner.submit(generation, move |_| {
        let _worker_exit = worker_exit;
        let Operation {
            id,
            name: _,
            kind,
            scope,
            reviewed_install,
        } = op;
        let progress_cancellation = worker_cancellation.clone();
        let progress_producer = producer.clone();
        let progress_id = id.clone();
        let mut throttle = ActivityPaintThrottle::default();
        let mut sink = |update: services::ProgressUpdate| {
            if update.app_id != progress_id.app_id {
                return;
            }
            let event = OpEvent::Progress {
                id: progress_id.clone(),
                progress: update.progress,
                message: update.message,
            };
            if let Some(event) = throttle.push(event, std::time::Instant::now())
                && progress_producer.send_progress(event).is_err()
            {
                progress_cancellation.cancel();
            }
        };
        let direct = reviewed_install.as_ref().filter(|review| review.direct);
        let result = match kind {
            OpKind::Install if let Some(review) = direct => services::install_streaming_from(
                &program,
                Some(&review.remote),
                &id.app_id,
                scope,
                &worker_cancellation,
                &mut sink,
            ),
            OpKind::Update if direct.is_some() => services::update_streaming(
                &program,
                Some(&id.app_id),
                scope,
                &worker_cancellation,
                &mut sink,
            ),
            OpKind::Install if reviewed_install.is_some() => services::install_review::install(
                reviewed_install.as_ref().expect("reviewed install"),
                &worker_cancellation,
                &mut sink,
            ),
            OpKind::Install if legacy_transport_fixture => services::install_streaming(
                &program,
                &id.app_id,
                scope,
                &worker_cancellation,
                &mut sink,
            ),
            OpKind::Install => Err("Installation requires a resolved permission review".to_owned()),
            OpKind::Update if reviewed_install.is_some() => services::install_review::install(
                reviewed_install.as_ref().expect("reviewed update"),
                &worker_cancellation,
                &mut sink,
            ),
            OpKind::Update if legacy_transport_fixture => services::update_streaming(
                &program,
                Some(&id.app_id),
                scope,
                &worker_cancellation,
                &mut sink,
            ),
            OpKind::Update => Err("Update requires a resolved permission review".to_owned()),
            OpKind::Uninstall => services::uninstall_streaming(
                &program,
                &id.app_id,
                scope,
                &worker_cancellation,
                &mut sink,
            ),
        };
        drop(sink);
        let terminal = match result {
            Ok(()) => OperationTerminal::Succeeded,
            Err(_) if worker_cancellation.is_cancelled_for_bridge() => OperationTerminal::Cancelled,
            Err(error) => OperationTerminal::Failed(error),
        };
        if let Some(event) = throttle.flush(std::time::Instant::now()) {
            let _ = producer.send_progress(event);
        }
        terminal
    });
    let progress_drained = Arc::new(AtomicBool::new(false));
    (
        OperationStream {
            receiver,
            close,
            cancellation,
            progress_drained: Arc::clone(&progress_drained),
        },
        completion,
        worker_exited,
    )
}

pub(crate) fn operation_terminal(
    result: Result<OperationTerminal, BlockingTaskError>,
) -> OperationTerminal {
    match result {
        Ok(terminal) => terminal,
        Err(BlockingTaskError::Busy) => OperationTerminal::Failed(
            "Package-change capacity was unavailable; the change was not attempted.".to_owned(),
        ),
        Err(BlockingTaskError::Cancelled) => OperationTerminal::Failed(
            "The package-change worker was cancelled; its result is unknown.".to_owned(),
        ),
        Err(BlockingTaskError::Shutdown) => OperationTerminal::Failed(
            "The package-change worker stopped during shutdown; its result is unknown.".to_owned(),
        ),
        Err(BlockingTaskError::Panicked) => OperationTerminal::Failed(
            "The package-change worker ended unexpectedly; its result is unknown.".to_owned(),
        ),
        Err(_) => OperationTerminal::Failed(
            "The package-change worker returned an unknown failure; its result is unknown."
                .to_owned(),
        ),
    }
}

// Submission returns separately owned progress and terminal handles so stream
// loss cannot discard the exact attempt's terminal custody.
pub(crate) fn operation_task(
    owner: &BlockingTaskOwner,
    op: Operation,
    program: PathBuf,
    cancellation: services::OperationCancellation,
) -> OperationTask {
    operation_task_internal(owner, op, program, cancellation, false)
}

fn operation_task_internal(
    owner: &BlockingTaskOwner,
    op: Operation,
    program: PathBuf,
    cancellation: services::OperationCancellation,
    legacy_transport_fixture: bool,
) -> OperationTask {
    let id = op.id.clone();
    let (stream, completion, worker_exited) =
        operation_stream(owner, op, program, cancellation, legacy_transport_fixture);
    let progress_drained = Arc::clone(&stream.progress_drained);
    OperationTask {
        id,
        progress: iced::Task::run(stream, |event| event),
        completion,
        progress_drained,
        worker_exited,
    }
}
