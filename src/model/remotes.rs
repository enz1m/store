//! Add-remote form and validation.

// Add-remote form (Flatpak Remotes / Manage remotes, cluster 1)
// ---------------------------------------------------------------------------

/// The live state of the "Add remote" form dialog.
///
/// Held on the `Store` only while the dialog is open (`Option::Some`), so a
/// closed dialog holds no draft. `name` and `url` are the two required fields;
/// `error` carries the last validation failure so the dialog can display it
/// inline **before** anything shells out to `flatpak remote-add`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AddRemoteForm {
    pub name: String,
    pub url: String,
    /// The submit error, if the last submit attempt was rejected.
    pub error: Option<String>,
    /// A submitted name conflict retained until an explicit decision or draft edit.
    pub conflict: Option<RemoteConflict>,
    /// An in-flight read or mutation that keeps the draft visible and prevents a duplicate change.
    pub busy: bool,
}

/// Retain the checked draft so confirmed replacement cannot use subsequent unreviewed edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteConflict {
    /// The repository name both the existing configuration and the draft use.
    pub name: String,
    /// The URL currently configured — what Replace would overwrite, and what a
    /// failed replacement is rolled back to.
    pub existing_url: String,
    /// The URL the user asked for.
    pub requested_url: String,
}

impl RemoteConflict {
    /// The disclosure shown in the retained form.
    ///
    /// Names both URLs because "this repository already exists" is exactly the
    /// message that let the old no-op pass for success: the user has to see
    /// that what is configured is not what they asked for.
    #[must_use]
    pub(crate) fn message(&self) -> String {
        format!(
            "{} already points at {}, not {}. Replace it, or choose a different name.",
            self.name,
            self.shown_existing_url(),
            self.shown_requested_url()
        )
    }

    /// A display-only URL with credentials masked; rollback retains the separate original value.
    #[must_use]
    pub(crate) fn shown_existing_url(&self) -> String {
        crate::services::redact_url_credentials(&self.existing_url)
    }

    /// The requested URL as it may be displayed, masked for the same reason.
    #[must_use]
    pub(crate) fn shown_requested_url(&self) -> String {
        crate::services::redact_url_credentials(&self.requested_url)
    }
}

/// Why an add-remote submission was rejected. Distinct variants (rather than a
/// single opaque bool) so the dialog can point the user at the offending field
/// and the reducer test can assert *which* rule fired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteFormError {
    /// The name field is empty or all-whitespace.
    EmptyName,
    /// The name contains a character `flatpak` will not accept in a remote id
    /// (whitespace or a path/URL separator).
    BadName,
    /// The URL field is empty or all-whitespace.
    EmptyUrl,
    /// The URL is not an `http(s)://` or `file://` URL — the common paste
    /// failure (a bare domain, a typo'd scheme). We refuse it rather than hand
    /// `flatpak` a value it will fail on obscurely.
    BadUrl,
}

impl RemoteFormError {
    /// A user-facing, single-line message for the dialog's inline error slot.
    pub(crate) fn message(self) -> &'static str {
        match self {
            RemoteFormError::EmptyName => "Enter a name for the remote.",
            RemoteFormError::BadName => {
                "Use letters, numbers, dots, dashes or underscores; do not start with a dash."
            }
            RemoteFormError::EmptyUrl => "Enter a repository URL.",
            RemoteFormError::BadUrl => "Enter a valid http://, https:// or file:// URL.",
        }
    }
}

/// Validate an add-remote (name, url) pair before it reaches `flatpak
/// remote-add`. The name is checked first so an empty form points at the name
/// field, matching the top-to-bottom reading order of the dialog.
///
/// A **bad URL is not silently accepted** — the brief's hard requirement. Only
/// `http://`, `https://` and `file://` are accepted; a bare host or a mistyped
/// scheme is rejected with [`RemoteFormError::BadUrl`].
pub(crate) fn validate_remote(name: &str, url: &str) -> Result<(), RemoteFormError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(RemoteFormError::EmptyName);
    }
    if crate::flatpak_args::validate_remote_name(name).is_err() {
        return Err(RemoteFormError::BadName);
    }
    let url = url.trim();
    if url.is_empty() {
        return Err(RemoteFormError::EmptyUrl);
    }
    if !is_valid_remote_url(url) {
        return Err(RemoteFormError::BadUrl);
    }
    Ok(())
}

/// Whether `url` is a scheme we will hand to `flatpak remote-add`.
///
/// Accepts `http://`, `https://` and `file://`, and requires a non-empty
/// authority/path after the scheme so `https://` alone is rejected.
pub(crate) fn is_valid_remote_url(url: &str) -> bool {
    let url = url.trim();
    // A .flatpakrepo file carries the repository's GPG key: fetched over plain
    // HTTP, anyone on the path could substitute it. Plain HTTP stays allowed
    // for repository URLs, whose commits are verified with a separate key.
    if url.starts_with("http://") && url.to_ascii_lowercase().ends_with(".flatpakrepo") {
        return false;
    }
    for scheme in ["https://", "http://", "file://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            return !rest.trim().is_empty();
        }
    }
    false
}
