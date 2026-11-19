//! Human, bounded translation of command failures.
//!
//! Raw argv stays at the process boundary. User-visible failures contain only
//! a fixed action phrase, a typed exit/transport category, and an optional
//! sanitized diagnostic excerpt.

// Hard ceiling for the complete accessibility label of a failure banner.
pub(crate) const RENDERED_FAILURE_CHAR_CAP: usize = 320;

/// Ceiling for the diagnostic portion, leaving room for action and category.
const DIAGNOSTIC_CHAR_CAP: usize = 200;

fn action_phrase(verb: &str) -> &'static str {
    match verb {
        "list" => crate::i18n::label("list your installed apps"),
        "remote-ls" => crate::i18n::label("check for updates"),
        "remotes" => crate::i18n::label("read your repositories"),
        "info" | "remote-info" => crate::i18n::label("read the app’s details"),
        "install" => crate::i18n::label("install the app"),
        "uninstall" => crate::i18n::label("remove the app"),
        "update" => crate::i18n::label("update the app"),
        "remote-add" => crate::i18n::label("add the repository"),
        // Replacement failure describes the existing repository that remains configured.
        "remote-modify" => crate::i18n::label("change the repository"),
        "remote-delete" => crate::i18n::label("remove the repository"),
        "override" => crate::i18n::label("change the app’s permissions"),
        _ => crate::i18n::label("run flatpak"),
    }
}

fn consume_csi(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for c in chars.by_ref() {
        if ('\u{40}'..='\u{7e}').contains(&c) {
            break;
        }
    }
}

fn consume_osc(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(c) = chars.next() {
        if c == '\u{7}' || c == '\u{9c}' {
            break;
        }
        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
            chars.next();
            break;
        }
    }
}

// Remove terminal escape sequences and collapse all remaining control and
// whitespace characters to ordinary single spaces.
pub(crate) fn sanitize_diagnostic(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // Seven-bit CSI and OSC.
            '\u{1b}' => match chars.next() {
                Some('[') => consume_csi(&mut chars),
                Some(']') => consume_osc(&mut chars),
                Some(_) | None => {}
            },
            // Eight-bit C1 CSI and OSC.
            '\u{9b}' => consume_csi(&mut chars),
            '\u{9d}' => consume_osc(&mut chars),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitize_diagnostic_bytes(raw: &[u8]) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut offset = 0;
    while offset < raw.len() {
        let byte = raw[offset];
        if (0x80..=0x9f).contains(&byte) {
            normalized.push(char::from(byte));
            offset += 1;
            continue;
        }
        match std::str::from_utf8(&raw[offset..]) {
            Ok(tail) => {
                normalized.push_str(tail);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    normalized.push_str(
                        std::str::from_utf8(&raw[offset..offset + valid])
                            .expect("UTF-8 validator identified a valid prefix"),
                    );
                    offset += valid;
                } else {
                    normalized.push('\u{fffd}');
                    offset += error.error_len().unwrap_or(1);
                }
            }
        }
    }
    sanitize_diagnostic(&normalized)
}

// Truncate at a Unicode scalar boundary and include an ellipsis when cut.
pub(crate) fn cap_chars(s: &str, cap: usize) -> String {
    if cap == 0 {
        return String::new();
    }
    if s.chars().count() <= cap {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(cap - 1).collect();
    out.push('…');
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExitCategory {
    Code(i32),
    Abnormal,
}

impl ExitCategory {
    fn of(status: std::process::ExitStatus) -> Self {
        match status.code() {
            Some(code) => Self::Code(code),
            None => Self::Abnormal,
        }
    }

    fn phrase(&self) -> String {
        match self {
            Self::Code(code) => crate::i18n::format(
                crate::i18n::localizer(),
                "flatpak exited with status {code}",
                &[("code", &(code))],
            ),
            Self::Abnormal => crate::i18n::label("flatpak exited abnormally").to_owned(),
        }
    }
}

// Typed failures produced before a usable process exit status exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TransportFailure {
    Cancelled,
    Spawn,
    ChildSetup,
    ReaderPanicked(&'static str),
    OutputLimit { stream: &'static str, bytes: usize },
    Read(&'static str),
    Deadline(u128),
    Wait,
}

impl TransportFailure {
    fn phrase(&self) -> String {
        match self {
            Self::Cancelled => crate::i18n::label("flatpak was cancelled").to_owned(),
            Self::Spawn => crate::i18n::label("flatpak could not be started").to_owned(),
            Self::ChildSetup => crate::i18n::label("flatpak process setup failed").to_owned(),
            Self::ReaderPanicked(stream) => crate::i18n::format(
                crate::i18n::localizer(),
                "flatpak {stream} reader failed",
                &[("stream", &(stream))],
            ),
            Self::OutputLimit { stream, bytes } => crate::i18n::format(
                crate::i18n::localizer(),
                "flatpak {stream} exceeded the {bytes}-byte response limit; output was truncated and was not parsed",
                &[("stream", &(stream)), ("bytes", &(bytes))],
            ),
            Self::Read(stream) => crate::i18n::format(
                crate::i18n::localizer(),
                "flatpak {stream} read failed",
                &[("stream", &(stream))],
            ),
            Self::Deadline(milliseconds) => crate::i18n::format(
                crate::i18n::localizer(),
                "flatpak read exceeded its {milliseconds} ms deadline",
                &[("milliseconds", &(milliseconds))],
            ),
            Self::Wait => crate::i18n::label("flatpak wait failed").to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FailureCategory {
    Exit(ExitCategory),
    Transport(TransportFailure),
}

impl FailureCategory {
    fn phrase(&self) -> String {
        match self {
            Self::Exit(status) => status.phrase(),
            Self::Transport(failure) => failure.phrase(),
        }
    }
}

// A bounded human category for a failed `flatpak` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlatpakCommandFailure {
    action: &'static str,
    category: FailureCategory,
    diagnostic: Option<String>,
}

impl FlatpakCommandFailure {
    pub(crate) fn from_stderr_bytes(
        args: &[String],
        status: std::process::ExitStatus,
        stderr: &[u8],
    ) -> Self {
        Self::from_clean_diagnostic(
            args,
            FailureCategory::Exit(ExitCategory::of(status)),
            sanitize_diagnostic_bytes(stderr),
        )
    }

    pub(crate) fn from_transport(
        args: &[String],
        failure: TransportFailure,
        diagnostic: &str,
    ) -> Self {
        Self::build(args, FailureCategory::Transport(failure), diagnostic)
    }

    fn build(args: &[String], category: FailureCategory, diagnostic: &str) -> Self {
        Self::from_clean_diagnostic(args, category, sanitize_diagnostic(diagnostic))
    }

    fn from_clean_diagnostic(args: &[String], category: FailureCategory, clean: String) -> Self {
        let verb = args.first().map_or("", String::as_str);
        let diagnostic = (!clean.is_empty()).then(|| cap_chars(&clean, DIAGNOSTIC_CHAR_CAP));
        Self {
            action: action_phrase(verb),
            category,
            diagnostic,
        }
    }

    pub(crate) fn message(&self) -> String {
        let category = self.category.phrase();
        let message = match &self.diagnostic {
            Some(diagnostic) => crate::i18n::format(
                crate::i18n::localizer(),
                "Could not {} — {category}: {diagnostic}",
                &[
                    ("arg0", &(crate::i18n::label(self.action))),
                    ("category", &(category)),
                    ("diagnostic", &(diagnostic)),
                ],
            ),
            None => crate::i18n::format(
                crate::i18n::localizer(),
                "Could not {} — {category}.",
                &[
                    ("arg0", &(crate::i18n::label(self.action))),
                    ("category", &(category)),
                ],
            ),
        };
        cap_chars(&message, RENDERED_FAILURE_CHAR_CAP)
    }
}
