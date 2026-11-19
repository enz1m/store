//! Metadata policy, routes, and HTTP errors.

use super::*;

// Record scope-specific argv so distinct installation queries remain independently observable.
#[derive(Default)]
pub(super) struct ScopeRecordingExecutor {
    calls: Mutex<Vec<Vec<String>>>,
    /// `(argv joined by ' ', reply)`. First match wins; an unmatched argv
    /// answers empty stdout with a zero exit, which is what `flatpak` does for
    /// a scope with nothing to report.
    replies: Vec<(String, Result<String, String>)>,
}

impl ScopeRecordingExecutor {
    pub(super) fn new(replies: &[(&str, Result<&str, &str>)]) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            replies: replies
                .iter()
                .map(|(argv, reply)| {
                    (
                        (*argv).to_owned(),
                        match reply {
                            Ok(out) => Ok((*out).to_owned()),
                            Err(error) => Err((*error).to_owned()),
                        },
                    )
                })
                .collect(),
        }
    }
}

impl FlatpakExecutor for ScopeRecordingExecutor {
    fn execute(&self, args: &[String]) -> Result<String, String> {
        let joined = args.join(" ");
        self.calls.lock().unwrap().push(args.to_vec());
        self.replies
            .iter()
            .find(|(argv, _)| *argv == joined)
            .map_or_else(|| Ok(String::new()), |(_, reply)| reply.clone())
    }
}
