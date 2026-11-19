// Flatpak executor boundary and bounded reads.

// Injectable process boundary for short Flatpak commands.
//
// Production uses [`ProcessFlatpakExecutor`]. Permission-route regression
// tests inject a parser here, below the real reducer and argv builder, without
// touching the user's Flatpak configuration.
// The classified outcome of one bounded metadata read attempt, used to decide
// whether a permission read may fall through to the next source.
pub(crate) enum MetadataReadStep {
    /// A cleanly terminated run produced metadata bytes.
    Present(String),
    /// A cleanly terminated run reported the app is absent from this scope, so
    /// the caller may try the next source.
    Absent,
    /// A terminal failure — cancellation, output overflow, process setup or
    /// read failure, a deadline, or incomplete cleanup — that must stop the
    /// whole read instead of falling through to another source.
    Terminal(String),
}

pub(crate) trait FlatpakExecutor: Send + Sync {
    fn execute(&self, args: &[String]) -> Result<String, String>;

    // Run a metadata read bounded by the shared `deadline` and classify the
    // outcome for source fallback.
    //
    // The default maps [`FlatpakExecutor::execute`]: a success is `Present`
    // and any error is a complete `Absent` for this scope, so an injected
    // executor that does not model transport failure keeps the historical
    // try-the-next-source behavior. The production executor overrides this to
    // classify a transport failure as `Terminal` and to honor the shared
    // deadline.
    fn read_metadata_step(&self, args: &[String], deadline: Instant) -> MetadataReadStep {
        let _ = deadline;
        match self.execute(args) {
            Ok(raw) => MetadataReadStep::Present(raw),
            Err(_) => MetadataReadStep::Absent,
        }
    }
}

const SHORT_READ_DEADLINE: Duration = Duration::from_secs(15);
/// One shared budget for the whole four-source permission metadata read, so a
/// stalled source cannot let the attempts sum to three separate deadlines.
const PERMISSION_READ_TOTAL_DEADLINE: Duration = Duration::from_secs(15);
/// The message for a permission read that found no metadata in any source. A
/// complete absence is still a failed read, never an empty declared set.
const PERMISSION_METADATA_ABSENT: &str = "flatpak metadata is not available for this application";
const APPSTREAM_READ_DEADLINE: Duration = Duration::from_secs(30);
const LIST_STDOUT_MAX_BYTES: usize = 4 * 1024 * 1024;
const METADATA_STDOUT_MAX_BYTES: usize = 1024 * 1024;
const REMOTES_STDOUT_MAX_BYTES: usize = 512 * 1024;
const DEFAULT_STDOUT_MAX_BYTES: usize = 256 * 1024;
const SHORT_READ_STDERR_MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug)]
struct ShortReadLimits {
    stdout: usize,
    stderr: usize,
    deadline: Duration,
}

impl ShortReadLimits {
    fn for_args(args: &[String]) -> Self {
        let verb = args.first().map_or("", String::as_str);
        let stdout = match verb {
            "list" | "remote-ls" => LIST_STDOUT_MAX_BYTES,
            "info" | "remote-info" | "override" => METADATA_STDOUT_MAX_BYTES,
            "remotes" => REMOTES_STDOUT_MAX_BYTES,
            _ => DEFAULT_STDOUT_MAX_BYTES,
        };
        let deadline = if verb == "update" && args.iter().any(|arg| arg == "--appstream") {
            APPSTREAM_READ_DEADLINE
        } else {
            SHORT_READ_DEADLINE
        };
        Self {
            stdout,
            stderr: SHORT_READ_STDERR_MAX_BYTES,
            deadline,
        }
    }
}

#[derive(Debug)]
struct BoundedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn translated_transport_failure(
    args: &[String],
    failure: TransportFailure,
    diagnostic: &str,
) -> String {
    FlatpakCommandFailure::from_transport(args, failure, diagnostic).message()
}

// Map a program to the absolute executable, then read it through the
// framework transport with a cleared, allowlisted environment and an explicit
// working directory. The transport owns pipe draining, the deadline, and exact
// process-group teardown; store cancellation is bridged onto its token.
fn run_bounded_flatpak(
    program: &Path,
    args: &[String],
    limits: ShortReadLimits,
    cancellation: &OperationCancellation,
) -> Result<BoundedOutput, String> {
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
    match process::run_owned(request, cancellation) {
        Ok(output) => Ok(BoundedOutput {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        }),
        Err(failure) => Err(translated_route_failure(args, &failure, &limits)),
    }
}

// The stream label the failure vocabulary already speaks.
fn transport_stream_name(stream: process_transport::OutputStream) -> &'static str {
    match stream {
        process_transport::OutputStream::Stdout => "stdout",
        process_transport::OutputStream::Stderr => "stderr",
    }
}

// Translate a transport route failure into the store's rendered failure copy,
// preserving the deadline and per-stream overflow phrasing existing callers
// match on. Cleanup and residual-group failures surface as a terminal error so
// they cannot pass as success.
fn translated_route_failure(
    args: &[String],
    failure: &process::RouteFailure,
    limits: &ShortReadLimits,
) -> String {
    use process_transport::ProcessErrorKind as Kind;
    let (transport, detail) = match failure {
        process::RouteFailure::Authority | process::RouteFailure::WorkingDirectory => {
            (TransportFailure::Spawn, None)
        }
        process::RouteFailure::CancellationBridgeUnavailable
        | process::RouteFailure::WorkerStartFailed => (TransportFailure::ChildSetup, None),
        process::RouteFailure::WorkerPanicked => (TransportFailure::Wait, None),
        process::RouteFailure::Transport(error) => match error.kind() {
            Kind::Cancelled(_) => (TransportFailure::Cancelled, None),
            Kind::Deadline => (
                TransportFailure::Deadline(limits.deadline.as_millis()),
                None,
            ),
            Kind::ExecutableAuthority | Kind::Spawn => (
                TransportFailure::Spawn,
                error.diagnostic().map(str::to_owned),
            ),
            Kind::Setup | Kind::StdinWrite | Kind::InvalidLimit(_) => (
                TransportFailure::ChildSetup,
                error.diagnostic().map(str::to_owned),
            ),
            Kind::Read(stream) => (TransportFailure::Read(transport_stream_name(*stream)), None),
            Kind::ReaderPanicked(stream) => (
                TransportFailure::ReaderPanicked(transport_stream_name(*stream)),
                None,
            ),
            Kind::WorkerPanicked => (TransportFailure::Wait, None),
            Kind::OutputLimitExceeded(stream) => (
                TransportFailure::OutputLimit {
                    stream: transport_stream_name(*stream),
                    bytes: match stream {
                        process_transport::OutputStream::Stdout => limits.stdout,
                        process_transport::OutputStream::Stderr => limits.stderr,
                    },
                },
                None,
            ),
            Kind::Wait | Kind::GroupSignal | Kind::ResidualCleanup | Kind::DirectChildCleanup => (
                TransportFailure::Wait,
                error.diagnostic().map(str::to_owned),
            ),
        },
    };
    translated_transport_failure(args, transport, detail.as_deref().unwrap_or(""))
}

#[derive(Debug)]
pub(crate) struct ProcessFlatpakExecutor {
    program: PathBuf,
    cancellation: OperationCancellation,
}

impl Default for ProcessFlatpakExecutor {
    fn default() -> Self {
        Self {
            program: PathBuf::from("flatpak"),
            cancellation: OperationCancellation::default(),
        }
    }
}

impl ProcessFlatpakExecutor {
    fn cancellable(cancellation: OperationCancellation) -> Self {
        Self {
            cancellation,
            ..Self::default()
        }
    }

}

impl FlatpakExecutor for ProcessFlatpakExecutor {
    fn execute(&self, args: &[String]) -> Result<String, String> {
        if self.cancellation.is_cancelled() {
            return Err(translated_transport_failure(
                args,
                TransportFailure::Cancelled,
                "",
            ));
        }
        let output = run_bounded_flatpak(
            &self.program,
            args,
            ShortReadLimits::for_args(args),
            &self.cancellation,
        )?;
        if !output.status.success() {
            return Err(FlatpakCommandFailure::from_stderr_bytes(
                args,
                output.status,
                &output.stderr,
            )
            .message());
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn read_metadata_step(&self, args: &[String], deadline: Instant) -> MetadataReadStep {
        // Cancellation stops the read; it is never a scope absence.
        if self.cancellation.is_cancelled() {
            return MetadataReadStep::Terminal(translated_transport_failure(
                args,
                TransportFailure::Cancelled,
                "",
            ));
        }
        let now = Instant::now();
        if now >= deadline {
            return MetadataReadStep::Terminal(translated_transport_failure(
                args,
                TransportFailure::Deadline(PERMISSION_READ_TOTAL_DEADLINE.as_millis()),
                "",
            ));
        }
        // Bound this source by whatever remains of the shared budget.
        let mut limits = ShortReadLimits::for_args(args);
        limits.deadline = deadline - now;
        match run_bounded_flatpak(&self.program, args, limits, &self.cancellation) {
            // A clean exit-zero run carries the metadata.
            Ok(output) if output.status.success() => {
                MetadataReadStep::Present(String::from_utf8_lossy(&output.stdout).to_string())
            }
            // A clean nonzero exit is a complete, explicit absence for this
            // scope; the caller may try the next source.
            Ok(_) => MetadataReadStep::Absent,
            // A transport failure (cancellation, overflow, process setup or
            // read failure, a deadline, or incomplete cleanup) stops the read
            // rather than masquerading as an absence.
            Err(error) => MetadataReadStep::Terminal(error),
        }
    }
}

// Run a no-operand Flatpak command. Commands containing external operands use
// [`crate::flatpak_args::FlatpakArgs`] before reaching this executor.
fn run_flatpak(executor: &dyn FlatpakExecutor, args: &[&str]) -> Result<String, String> {
    executor.execute(
        &args
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect::<Vec<_>>(),
    )
}

fn run_flatpak_args_with(
    executor: &dyn FlatpakExecutor,
    args: &[String],
) -> Result<String, String> {
    executor.execute(args)
}

// Parse a tab-separated `flatpak --columns=...` payload into a vector of
// column-indexed row vectors. `flatpak` prints tabs between columns and a
// trailing newline. Empty rows are skipped.
fn parse_columns(raw: &str) -> Vec<Vec<String>> {
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split('\t')
                .map(|cell| cell.trim().to_owned())
                .collect()
        })
        .collect()
}

// Flatpak sizes use decimal SI units; malformed or absent values contribute zero bytes.
pub(crate) fn parse_size_bytes(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() || s == "-" {
        return 0;
    }
    // Split on the first ASCII space; number on the left, unit on the
    // right. Fallback: if there's no space, try to split at the last
    // digit / decimal.
    let (num_str, unit_str) = match s.find(' ') {
        Some(i) => (&s[..i], s[i + 1..].trim()),
        None => {
            // Split at the first non-digit-non-dot char.
            match s.find(|c: char| !c.is_ascii_digit() && c != '.') {
                Some(i) => (&s[..i], s[i..].trim()),
                None => (s, ""),
            }
        }
    };
    let value: f64 = match num_str.parse() {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let multiplier: f64 = match unit_str.to_ascii_lowercase().as_str() {
        "" | "b" | "bytes" | "byte" => 1.0,
        "kb" | "kib" | "k" => 1_000.0,
        "mb" | "mib" | "m" => 1_000_000.0,
        "gb" | "gib" | "g" => 1_000_000_000.0,
        "tb" | "tib" | "t" => 1_000_000_000_000.0,
        _ => return 0, // Unknown unit — fail closed rather than silently miscompute.
    };
    (value * multiplier) as u64
}

/// Format storage using the shared decimal SI policy, including a visible zero-size total.
pub(crate) fn format_size_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 MB".to_owned();
    }
    enzui_widgets::file_browser::format_bytes(bytes)
}
