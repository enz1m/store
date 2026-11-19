// Progress updates, cancellation, and streaming runs.

/// A streamed progress update from a running flatpak operation.
#[derive(Clone, Debug)]
pub(crate) struct ProgressUpdate {
    pub app_id: String,
    pub progress: f32,
    pub message: String,
}

/// Sink for streamed progress updates.
pub(crate) type ProgressSink<'a> = &'a mut dyn FnMut(ProgressUpdate);

/// Cooperative cancellation shared by an owner and its blocking child.
/// The process loop observes both explicit requests and cancellation of the
/// bounded worker submission before performing whole-group teardown.
#[derive(Clone, Default)]
pub(crate) struct OperationCancellation {
    requested: Arc<AtomicBool>,
    blocking_context: Option<enzui_core::blocking_task::BlockingTaskContext>,
}

impl std::fmt::Debug for OperationCancellation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperationCancellation")
            .field("requested", &self.requested.load(Ordering::Acquire))
            .field("worker_bound", &self.blocking_context.is_some())
            .finish()
    }
}

impl OperationCancellation {
    pub(crate) fn cancel(&self) {
        self.requested.store(true, Ordering::Release);
    }

    pub(crate) fn with_blocking_context(
        &self,
        context: enzui_core::blocking_task::BlockingTaskContext,
    ) -> Self {
        Self {
            requested: Arc::clone(&self.requested),
            blocking_context: Some(context),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.requested.load(Ordering::Acquire)
            || self
                .blocking_context
                .as_ref()
                .is_some_and(enzui_core::blocking_task::BlockingTaskContext::is_cancelled)
    }

    pub(crate) fn is_cancelled_for_bridge(&self) -> bool {
        self.is_cancelled()
    }
}

pub(crate) const OUTPUT_LINE_MAX_BYTES: usize = 8 * 1024;

/// Parse a Flatpak progress percentage and its leading phase text, excluding rendered bar glyphs.
pub(crate) fn parse_flatpak_progress(line: &str) -> Option<(f32, Option<String>)> {
    // Find the last occurrence of a bare number immediately followed by '%'.
    // This avoids false positives from size strings like "389.0 MB".
    let mut pct_end = None;
    let chars: Vec<char> = line.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '%' && i > 0 && chars[i - 1].is_ascii_digit() {
            pct_end = Some(i);
        }
    }
    let pct_pos = pct_end?;

    // Walk backwards from the digit before '%' to capture the full integer.
    let mut start = pct_pos - 1;
    while start > 0 && chars[start - 1].is_ascii_digit() {
        start -= 1;
    }
    let num_str: String = chars[start..pct_pos].iter().collect();
    let pct: u32 = num_str.parse().ok()?;
    let progress = (pct as f32 / 100.0).clamp(0.0, 1.0);

    let phase = extract_phase(line);

    Some((progress, phase))
}

// Pull the leading "Installing…", "Updating…", "Downloading…" etc. from a
// flatpak progress line. Returns `None` when the line has no recognisable
// phase prefix.
fn extract_phase(line: &str) -> Option<String> {
    // Unicode blocks and multi-space separators end phase text before the progress display.
    let mut last_nonws: usize = 0;
    let mut ws_run: usize = 0;
    let mut found_gap = false;
    for (i, ch) in line.char_indices() {
        if ('\u{2580}'..='\u{259F}').contains(&ch) {
            found_gap = true;
            break;
        }
        if ch.is_whitespace() {
            ws_run += 1;
            if ws_run >= 2 {
                found_gap = true;
                break;
            }
        } else {
            ws_run = 0;
            last_nonws = i + ch.len_utf8();
        }
    }
    let boundary = if found_gap { last_nonws } else { line.len() };
    let phase = line[..boundary].trim().to_owned();
    if phase.is_empty() { None } else { Some(phase) }
}

// Streaming mutations share one operation owner for progress and cancellation.

// Only an authorized MutationScope reaches mutation argument construction.

// Application operands follow --; installation scope remains an authorized option.
pub(crate) fn mutation_args(
    subcommand: &str,
    app_id: Option<&str>,
    scope: MutationScope,
) -> Result<Vec<String>, String> {
    let mut args = crate::flatpak_args::FlatpakArgs::new(subcommand);
    args.options([scope.option(), "-y"]);
    if let Some(id) = app_id {
        args.app_id(id)?;
    }
    Ok(args.finish())
}

/// Streaming install: runs flatpak with piped stdout, parses progress lines,
/// and pushes `ProgressUpdate`s into `sink` as they arrive.
pub(crate) fn install_streaming(
    program: &Path,
    app_id: &str,
    scope: MutationScope,
    cancellation: &OperationCancellation,
    sink: ProgressSink<'_>,
) -> Result<(), String> {
    install_streaming_from(program, None, app_id, scope, cancellation, sink)
}

/// Streaming install from a named remote, so an app two remotes offer
/// installs from the one Store resolved rather than whichever Flatpak picks.
pub(crate) fn install_streaming_from(
    program: &Path,
    remote: Option<&str>,
    app_id: &str,
    scope: MutationScope,
    cancellation: &OperationCancellation,
    sink: ProgressSink<'_>,
) -> Result<(), String> {
    let mut args = crate::flatpak_args::FlatpakArgs::new("install");
    args.options([scope.option(), "-y"]);
    if let Some(remote) = remote {
        args.remote(remote)?;
    }
    args.app_id(app_id)?;
    run_flatpak_streaming(program, &args.finish(), app_id, cancellation, sink)
}

/// Streaming update: runs flatpak with piped stdout, parses progress lines,
/// and pushes `ProgressUpdate`s into `sink` as they arrive.
pub(crate) fn update_streaming(
    program: &Path,
    app_id: Option<&str>,
    scope: MutationScope,
    cancellation: &OperationCancellation,
    sink: ProgressSink<'_>,
) -> Result<(), String> {
    let key = app_id.unwrap_or("*");
    let args = mutation_args("update", app_id, scope)?;
    run_flatpak_streaming(program, &args, key, cancellation, sink)
}

/// Streaming uninstall. Uninstall emits far fewer progress lines than an
/// install, but routing it through the same path means the Activity section
/// shows one uniform row shape for every operation kind.
pub(crate) fn uninstall_streaming(
    program: &Path,
    app_id: &str,
    scope: MutationScope,
    cancellation: &OperationCancellation,
    sink: ProgressSink<'_>,
) -> Result<(), String> {
    let args = mutation_args("uninstall", Some(app_id), scope)?;
    run_flatpak_streaming(program, &args, app_id, cancellation, sink)
}

/// Upper bounds for a streaming mutation. Progress text is line-parsed and
/// discarded, but the transport retains complete output up to these limits; the
/// stderr bound is generous enough that a failing operation's diagnostic
/// reaches translation intact before the per-message cap applies.
const STREAM_STDOUT_MAX_BYTES: usize = 8 * 1024 * 1024;
const STREAM_STDERR_MAX_BYTES: usize = 4 * 1024 * 1024;
const STREAM_DEADLINE: Duration = Duration::from_secs(60 * 60);

// Parse one output line into a progress update for `key`, if it carries a
// percentage.
fn progress_from_line(line: &[u8], key: &str) -> Option<ProgressUpdate> {
    let line = String::from_utf8_lossy(line);
    parse_flatpak_progress(&line).map(|(progress, phase)| ProgressUpdate {
        app_id: key.to_owned(),
        progress,
        message: phase.unwrap_or_else(|| "Working…".to_owned()),
    })
}

// Run one mutating command over the framework transport, parsing progress
// lines from streamed stdout chunks and publishing each into `sink` on the
// calling thread. Diagnostic and progress retention stay bounded even when a
// pipe or the iced consumer is much slower than the child.
fn run_flatpak_streaming(
    program: &Path,
    args: &[String],
    key: &str,
    cancellation: &OperationCancellation,
    sink: ProgressSink<'_>,
) -> Result<(), String> {
    let limits = ShortReadLimits {
        stdout: STREAM_STDOUT_MAX_BYTES,
        stderr: STREAM_STDERR_MAX_BYTES,
        deadline: STREAM_DEADLINE,
    };
    let executable = process::resolve_authority(program)
        .map_err(|failure| translated_route_failure(args, &failure, &limits))?;
    let request = process::build_request(
        executable,
        args,
        process_transport::StdinPolicy::Null,
        limits.stdout,
        limits.stderr,
        limits.deadline,
        process::FLATPAK_ENV_ALLOWLIST,
    )
    .map_err(|failure| translated_route_failure(args, &failure, &limits))?;

    let mut line: Vec<u8> = Vec::with_capacity(256);
    let mut oversized = false;
    let output = process::run_owned_streaming(request, cancellation, |chunk| {
        if chunk.stream != process_transport::OutputStream::Stdout {
            return;
        }
        for &byte in &chunk.bytes {
            if byte == b'\n' {
                if !oversized && let Some(update) = progress_from_line(&line, key) {
                    sink(update);
                }
                line.clear();
                oversized = false;
            } else if line.len() < OUTPUT_LINE_MAX_BYTES {
                line.push(byte);
            } else {
                oversized = true;
            }
        }
    });
    let output = match output {
        Ok(output) => output,
        Err(failure) => return Err(translated_route_failure(args, &failure, &limits)),
    };
    if !line.is_empty()
        && !oversized
        && let Some(update) = progress_from_line(&line, key)
    {
        sink(update);
    }
    if !output.status.success() {
        return Err(
            FlatpakCommandFailure::from_stderr_bytes(args, output.status, &output.stderr).message(),
        );
    }
    Ok(())
}
