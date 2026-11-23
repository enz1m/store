/// Flatpak installation containing an installed app, used to route and authorize mutations.
/// Catalog rows retain Unknown even when their ID matches an installed app;
/// mutating controls must obtain scope from the installed-app read.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum InstallScope {
    /// No installation was reported: a catalogue row, or an installed row
    /// whose `installation` cell came back empty.
    #[default]
    Unknown,
    /// The per-user installation, selected with `--user`.
    User,
    /// The default system-wide installation, selected with `--system`.
    System,
    /// An extra system-wide installation, selected with
    /// `--installation=NAME`. The name is carried verbatim because it is the
    /// operand flatpak matches on.
    Named(String),
}

/// Keep the suppressed-control label and its failure reason as one disclosure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Refusal {
    /// The chip text, and therefore the accessible name of the node that
    /// stands in for the suppressed control.
    pub label: &'static str,
    /// The full sentence, shown as the operation's error if the action is
    /// requested despite being suppressed.
    pub reason: &'static str,
}

/// Disclose consequences an allowed action cannot cover, separately from a refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Notice {
    /// The chip text, and therefore the accessible name of the node beside the
    /// control it qualifies.
    pub label: &'static str,
    /// The full sentence, shown in the confirmation that precedes the action.
    pub reason: &'static str,
}

/// Removing the user copy of an app that is **also** installed system-wide
/// leaves the system copy installed.
///
/// "Remove" on a dual-installed app removes the user copy only, so it is a
/// partial removal. Without this the operation succeeds, the row disappears
/// from the Library for one frame, the authoritative re-read puts it straight
/// back as the system copy, and the user is left to work out whether the
/// removal failed. It did not fail; it did exactly half of what the word
/// promises, and the honest thing is to say so before the confirmation rather
/// than after the refresh.
pub(crate) const SYSTEM_COPY_REMAINS: Notice = Notice {
    label: "System copy remains",
    reason: "A system-wide copy of this app will stay installed: this removes \
             only your user installation.",
};

/// Disclose a system-managed installation consistently across app and repository surfaces.
pub(crate) const SYSTEM_MANAGED: Refusal = Refusal {
    label: "Managed by system",
    reason: "Managed by system: this app is installed in a custom system-wide \
             Flatpak installation, and Store can only change your user \
             installation and the default system installation.",
};

/// Refuse mutation when the installed ref has no authoritative installation scope.
pub(crate) const SCOPE_UNKNOWN: Refusal = Refusal {
    label: "Installation unknown",
    reason: "Store could not determine which Flatpak installation this app is in, \
             so it will not guess. Reload your installed apps; if this persists, \
             manage the app with the flatpak command line.",
};

/// Refuse installation when every positively matched source is system-wide.
pub(crate) const SYSTEM_REMOTE_ONLY: Refusal = Refusal {
    label: "System repository",
    reason: "This app is only offered by a system-wide repository, and Store can \
             only install into your user installation. Add the repository with \
             --user, or install it with the flatpak command line.",
};

/// Defer installation while repository authority is still loading.
pub(crate) const REPOSITORIES_LOADING: Refusal = Refusal {
    label: "Checking repositories…",
    reason: "Store is still reading your configured Flatpak repositories and \
             does not yet know which of them can supply this app.",
};

/// Report repository-read failure with the actual refresh action available to retry it.
pub(crate) const REPOSITORIES_UNAVAILABLE: Refusal = Refusal {
    label: "Repositories unavailable",
    reason: "Store could not read your configured Flatpak repositories, so it \
             cannot tell which of them can supply this app. Use Retry on the \
             Repositories page and try again.",
};

/// Refuse installation when no loaded repository provides an authoritative matching source.
pub(crate) const REPOSITORY_UNKNOWN: Refusal = Refusal {
    label: "Repository unknown",
    reason: "No configured Flatpak repository matches this app's source, so \
             Store cannot tell where it would be installed from. Add the \
             repository to your user installation, then try again.",
};

/// Preserve loading, failed and complete repository reads as distinct states.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum RemoteLoad {
    /// The read was dispatched and has not answered. The honest production
    /// state at boot, and the reason a store must *say* its repositories
    /// settled rather than being assumed to have.
    #[default]
    Loading,
    /// The read failed, carrying flatpak's own diagnostic.
    Failed(String),
    /// The read answered. The accompanying list is authoritative, including
    /// when it is empty.
    Loaded,
}

impl RemoteLoad {
    /// The diagnostic to disclose, if the read failed.
    #[must_use]
    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(reason) => Some(reason.as_str()),
            Self::Loading | Self::Loaded => None,
        }
    }
}

/// Carry repository state and rows together so authorization cannot lose the read outcome.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RepositoryAnswer<'a> {
    /// Whether the read is in flight, failed, or answered.
    pub load: &'a RemoteLoad,
    /// The repositories it returned. Meaningful only when `load` is
    /// [`RemoteLoad::Loaded`], and authoritative then even when empty.
    pub remotes: &'a [RemoteInfo],
}

impl<'a> RepositoryAnswer<'a> {
    /// Whether a fresh install of a row with this `origin` may run, and where.
    #[must_use]
    pub(crate) fn authorization(self, origin: &str) -> Authorization {
        install_authorization(origin, self.load, self.remotes)
    }
}

/// Match catalogue sources by URL as well as configurable local remote name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CatalogRepository {
    /// The conventional remote name, which is also the `origin` this app's
    /// catalogue rows carry.
    pub origin: &'static str,
    /// The repository URL, compared after [`normalise_repository_url`].
    pub url: &'static str,
}

/// Flathub’s public catalogue.
pub(crate) const FLATHUB_REPOSITORY: CatalogRepository = CatalogRepository {
    origin: "flathub",
    // The same URL `fixture_remotes` declares and `flatpak remotes
    // --columns=url` prints for a stock Flathub remote.
    url: "https://dl.flathub.org/repo/",
};

pub(crate) const ENZIMOS_REPOSITORY: CatalogRepository = CatalogRepository {
    origin: "enzimos",
    url: "https://flatpak.enzim.dev/repo/",
};

const KNOWN_CATALOG_REPOSITORIES: &[CatalogRepository] =
    &[FLATHUB_REPOSITORY, ENZIMOS_REPOSITORY];

pub(crate) fn matches_catalog_origin(remote: &RemoteInfo, origin: &str) -> bool {
    let url_matches = KNOWN_CATALOG_REPOSITORIES
        .iter()
        .find(|repository| repository.origin == origin)
        .is_some_and(|repository| normalise_repository_url(&remote.url) ==
            normalise_repository_url(repository.url));
    if origin == ENZIMOS_REPOSITORY.origin {
        url_matches && !remote.options.split(',').any(|flag| {
            matches!(flag.trim(), "disabled" | "noenumerate" | "no-gpg-verify")
        })
    } else {
        remote.name == origin || url_matches
    }
}

/// Compare two repository URLs the way two spellings of one repository should
/// compare: case-insensitively, ignoring a trailing slash and surrounding
/// space.
///
/// Deliberately not a URL parse. The comparison is between a constant this
/// crate ships and a string flatpak printed, and both are already canonical
/// apart from those three differences; a parser would add failure modes
/// without adding matches.
#[must_use]
pub(crate) fn normalise_repository_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Mask URL userinfo in composed disclosures, retaining a marker for credential-only conflicts; quoted command diagnostics are separate.
#[must_use]
pub(crate) fn redact_url_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    // The authority ends at the first path, query or fragment delimiter.
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    // RFC 3986 splits userinfo from host at the LAST `@` in the authority, so
    // a password containing `@` cannot hide the boundary.
    match authority.rfind('@') {
        Some(at) => format!("{scheme}://***@{}{tail}", &authority[at + 1..]),
        None => url.to_owned(),
    }
}

// Keep repository identity and requested action attached to backend diagnostics.
fn repository_failure(action: &str, name: &str, diagnostic: &str) -> String {
    format!("Could not {action} {name}: {}", subordinate(diagnostic))
}

// Fold a backend diagnostic into a subordinate clause.
//
// [`flatpak_failure`](crate::services::flatpak_failure) already renders a
// process failure as a whole sentence — "Could not change the repository —
// exit 1: …" — because most callers surface it directly. Nested inside
// [`repository_failure`]'s own sentence that stutters, so the leading
// "Could not " is dropped.
//
// Only that prefix, and only when it is there: it is this app's own
// translation wording, not text from the backend. A raw executor diagnostic
// (an injected executor, or `flatpak`'s stderr quoted through the
// bounded excerpt) has no such prefix and is passed through byte for byte.
fn subordinate(diagnostic: &str) -> &str {
    diagnostic.strip_prefix("Could not ").unwrap_or(diagnostic)
}

/// The user-facing name of the action a retained conflict's Replace button
/// starts, used by every message that reports its outcome.
const REPLACE_ACTION: &str = "replace";

/// The user-facing name of the action the form's affirmative button starts.
const ADD_ACTION: &str = "add";

fn invalid_repository_url(action: &str, name: &str) -> String {
    repository_failure(action, name, "that is not a valid repository URL")
}
