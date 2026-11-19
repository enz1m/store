//! Owned-process routes over the framework process transport.
//!
//! The transport owns spawn, bounded pipe draining, the total deadline, and
//! exact process-group TERM/KILL/reap; this module resolves executable
//! authority once against an explicit search path, clears the child environment
//! down to a route allowlist, pins an owner-private working directory (failing
//! the route when none is available), and bridges the store's cooperative
//! cancellation onto the transport's token (failing the route rather than
//! running uncancellable work if the bridge cannot start).

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::time::Duration;

use enzui_core::cancellation::{CancelCause, CancelToken};
use enzui_core::runtime_dir::{self, RuntimeDir, RuntimeDirError};
use enzui_services::process_transport::{
    self, EnvPolicy, ProcessError, ProcessOutput, ProcessOutputChunk, ProcessRequest, StdinPolicy,
};

use super::OperationCancellation;

/// Interval between cancellation-flag observations while a helper runs.
const BRIDGE_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Streamed-chunk channel depth. Chunk delivery is best effort, so a shallow
/// bound keeps a slow consumer from retaining unbounded transient output.
const CHUNK_CHANNEL_CAPACITY: usize = 16;

// Variables the `flatpak` client reads to locate installations and reach the
// system helper. Everything outside this set is cleared before spawn.
pub(crate) const FLATPAK_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_DATA_HOME",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_SESSION_TYPE",
    "LANG",
    "LC_ALL",
    "LC_NUMERIC",
];

#[cfg(test)]
thread_local! {
    // When set on the calling thread under test, the cancellation bridge
    // reports a spawn failure so the fail-closed path can be exercised without
    // a real thread-creation limit. Thread-local so a concurrent route on
    // another test thread is unaffected.
    pub(crate) static FAIL_CANCEL_BRIDGE_SPAWN: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

// A route could not start or complete a helper.
#[derive(Debug)]
pub(crate) enum RouteFailure {
    /// The program was not an executable absolute path or a bare name resolvable
    /// through the supplied search path of absolute directories.
    Authority,
    /// No owner-private working directory was available to run the child in.
    WorkingDirectory,
    /// The cancellation bridge could not start, so the child was never run
    /// rather than run without a cancellation path.
    CancellationBridgeUnavailable,
    /// The streaming transport worker thread could not be spawned, so no child
    /// was started.
    WorkerStartFailed,
    /// The transport returned a typed terminal failure.
    Transport(ProcessError),
    /// The streaming worker thread unwound; the transport itself never returns
    /// through a panic, so this marks a broken invariant rather than helper
    /// behaviour.
    WorkerPanicked,
}

// Resolve `program` against the launcher's current `PATH` snapshot.
pub(crate) fn resolve_authority(program: &Path) -> Result<PathBuf, RouteFailure> {
    resolve_authority_in(program, std::env::var_os("PATH").as_deref())
}

// Resolve `program` to the absolute, canonical executable the transport will
// spawn, using only `search_path` for a bare name.
//
// An absolute path must itself be an executable regular file and is returned
// canonicalized. A bare name is looked up through `search_path`, whose entries
// must all be nonempty absolute directories; the first executable match is
// canonicalized. A relative multi-component path, an absent candidate, or a
// search path with a relative component is refused before a child is created.
// Because the result is always absolute and canonical, no later working
// directory or ambient `PATH` change can redirect the spawn.
pub(crate) fn resolve_authority_in(
    program: &Path,
    search_path: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, RouteFailure> {
    if program.is_absolute() {
        return canonical_executable(program);
    }
    if !is_bare_name(program) {
        return Err(RouteFailure::Authority);
    }
    let search_path = search_path.ok_or(RouteFailure::Authority)?;
    let directories: Vec<PathBuf> = std::env::split_paths(search_path).collect();
    if directories.is_empty()
        || directories
            .iter()
            .any(|directory| directory.as_os_str().is_empty() || !directory.is_absolute())
    {
        return Err(RouteFailure::Authority);
    }
    for directory in directories {
        let candidate = directory.join(program);
        if is_executable_file(&candidate) {
            return std::fs::canonicalize(&candidate).map_err(|_| RouteFailure::Authority);
        }
    }
    Err(RouteFailure::Authority)
}

// Accept an absolute path only when it is an executable regular file, and
// return its canonical form.
fn canonical_executable(program: &Path) -> Result<PathBuf, RouteFailure> {
    if !is_executable_file(program) {
        return Err(RouteFailure::Authority);
    }
    std::fs::canonicalize(program).map_err(|_| RouteFailure::Authority)
}

fn is_bare_name(program: &Path) -> bool {
    let mut components = program.components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

// Build a request with a cleared, allowlisted environment and an owner-private
// working directory, or fail the route if no safe directory is available.
pub(crate) fn build_request(
    executable: PathBuf,
    args: &[String],
    stdin: StdinPolicy,
    stdout_limit: usize,
    stderr_limit: usize,
    deadline: Duration,
    env_allowlist: &[&str],
) -> Result<ProcessRequest, RouteFailure> {
    let working_directory = private_working_directory()?;
    let mut request = ProcessRequest::new(executable, stdin, stdout_limit, stderr_limit, deadline)
        .args(args.iter().cloned());
    debug_assert!(matches!(request.env_policy(), &EnvPolicy::Cleared));
    let mut language = None;
    for key in env_allowlist {
        if let Some(value) = std::env::var_os(key) {
            // LC_ALL would override LC_NUMERIC below; carry it as LANG instead
            // so messages stay translated.
            if *key == "LC_ALL" {
                language = Some(value);
                continue;
            }
            request = request.env(*key, value);
        }
    }
    if let Some(language) = language {
        request = request.env("LANG", language);
    }
    // Sizes and other numbers are parsed from flatpak's output: keep them in
    // the C format ("389.0 MB", not "389,0 MB") whatever the user's locale.
    if env_allowlist.contains(&"LANG") {
        request = request.env("LC_NUMERIC", "C");
    }
    Ok(request.current_dir(working_directory))
}

// The owner-private directory the child runs in, from the framework runtime
// directory authority.
fn private_working_directory() -> Result<PathBuf, RouteFailure> {
    working_directory_from(runtime_dir::runtime_dir())
}

// Accept only a runtime directory the authority proved owner-private; a
// refused insecure `$XDG_RUNTIME_DIR` or an unavailable directory fails the
// route rather than running the child in a shared or unverified location.
pub(crate) fn working_directory_from(
    resolved: Result<RuntimeDir, RuntimeDirError>,
) -> Result<PathBuf, RouteFailure> {
    match resolved {
        Ok(directory) if !directory.is_refusal() => Ok(directory.path().to_path_buf()),
        _ => Err(RouteFailure::WorkingDirectory),
    }
}

// Run one helper to completion, bridging cooperative cancellation onto the
// transport token.
pub(crate) fn run_owned(
    request: ProcessRequest,
    cancellation: &OperationCancellation,
) -> Result<ProcessOutput, RouteFailure> {
    let token = CancelToken::new();
    if cancellation.is_cancelled_for_bridge() {
        token.cancel(CancelCause::Cancelled);
    }
    let done = Arc::new(AtomicBool::new(false));
    let watcher = spawn_cancel_bridge(cancellation.clone(), token.share(), Arc::clone(&done))?;
    let result = process_transport::run(request, &token);
    done.store(true, Ordering::Release);
    if let Some(watcher) = watcher {
        let _ = watcher.join();
    }
    result.map_err(RouteFailure::Transport)
}

// Run one helper while draining best-effort output chunks to `on_chunk` on the
// calling thread, bridging cancellation between chunk observations.
pub(crate) fn run_owned_streaming(
    request: ProcessRequest,
    cancellation: &OperationCancellation,
    mut on_chunk: impl FnMut(ProcessOutputChunk),
) -> Result<ProcessOutput, RouteFailure> {
    let (sender, receiver) = sync_channel::<ProcessOutputChunk>(CHUNK_CHANNEL_CAPACITY);
    let token = CancelToken::new();
    if cancellation.is_cancelled_for_bridge() {
        token.cancel(CancelCause::Cancelled);
    }
    let worker_token = token.share();
    let worker = std::thread::Builder::new()
        .name("flatpak-stream-owner".into())
        .spawn(move || process_transport::run_streaming(request, &worker_token, sender))
        .map_err(|_| RouteFailure::WorkerStartFailed)?;
    let drain = catch_unwind(AssertUnwindSafe(|| {
        loop {
            match receiver.recv_timeout(BRIDGE_POLL_INTERVAL) {
                Ok(chunk) => on_chunk(chunk),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if cancellation.is_cancelled_for_bridge() {
                token.cancel(CancelCause::Cancelled);
            }
        }
    }));
    if drain.is_err() {
        token.cancel(CancelCause::Cancelled);
    }
    let worker_result = worker.join();
    if let Err(payload) = drain {
        resume_unwind(payload);
    }
    match worker_result {
        Ok(result) => result.map_err(RouteFailure::Transport),
        Err(_) => Err(RouteFailure::WorkerPanicked),
    }
}

// Spawn a watcher that copies the store's cancellation onto the transport
// token. `Ok(None)` when cancellation was already observed, so no thread is
// created for an already-cancelled run; an error when the watcher cannot start,
// so the caller declines to run uncancellable work.
fn spawn_cancel_bridge(
    cancellation: OperationCancellation,
    token: CancelToken,
    done: Arc<AtomicBool>,
) -> Result<Option<std::thread::JoinHandle<()>>, RouteFailure> {
    if token.is_terminal() {
        return Ok(None);
    }
    #[cfg(test)]
    if FAIL_CANCEL_BRIDGE_SPAWN.with(std::cell::Cell::get) {
        return Err(RouteFailure::CancellationBridgeUnavailable);
    }
    std::thread::Builder::new()
        .name("flatpak-cancel-bridge".into())
        .spawn(move || {
            while !done.load(Ordering::Acquire) {
                if cancellation.is_cancelled_for_bridge() {
                    token.cancel(CancelCause::Cancelled);
                    return;
                }
                std::thread::sleep(BRIDGE_POLL_INTERVAL);
            }
        })
        .map(Some)
        .map_err(|_| RouteFailure::CancellationBridgeUnavailable)
}
