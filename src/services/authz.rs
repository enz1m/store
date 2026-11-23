// Authorization and install authorization.

/// Whether an action may run, and in which installation, or why it may not.
///
/// The single vocabulary the **view** and the **reducer** both speak, which is
/// what makes "never show a clickable control that is known to target a scope
/// where it cannot succeed" structural rather than a convention. A surface
/// renders a button for [`Authorization::Allowed`] and
/// [`Refusal::label`] for [`Authorization::Refused`]; the reducer starts work
/// for the first and records [`Refusal::reason`] for the second. Neither can
/// reach a scope the other did not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Authorization {
    /// The action may run, against this installation.
    Allowed(MutationScope),
    /// The action is suppressed, for this stated reason.
    Refused(Refusal),
}

impl Authorization {
    /// The installation to run in, or `None` when the action is suppressed.
    #[must_use]
    pub(crate) fn scope(self) -> Option<MutationScope> {
        match self {
            Self::Allowed(scope) => Some(scope),
            Self::Refused(_) => None,
        }
    }

    /// Whether a control may be offered for this action.
    #[must_use]
    pub(crate) fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed(_))
    }

    /// The disclosure to render in place of the suppressed control.
    #[must_use]
    pub(crate) fn refusal(self) -> Option<Refusal> {
        match self {
            Self::Allowed(_) => None,
            Self::Refused(refusal) => Some(refusal),
        }
    }
}

/// Authorize a fresh install only when a loaded repository positively matches the origin.
/// Known catalog origins match by name or published URL; other origins match by name only.
/// User scope wins a positive match; loading, failed, missing, and system-only sources refuse.
#[must_use]
pub(crate) fn install_authorization(
    origin: &str,
    load: &RemoteLoad,
    remotes: &[RemoteInfo],
) -> Authorization {
    match load {
        RemoteLoad::Loading => return Authorization::Refused(REPOSITORIES_LOADING),
        RemoteLoad::Failed(_) => return Authorization::Refused(REPOSITORIES_UNAVAILABLE),
        RemoteLoad::Loaded => {}
    }
    let origin = origin.trim();
    if origin.is_empty() {
        return Authorization::Refused(REPOSITORY_UNKNOWN);
    }
    let matched: Vec<&RemoteInfo> = remotes
        .iter()
        .filter(|remote| matches_catalog_origin(remote, origin))
        .collect();
    if matched.is_empty() {
        return Authorization::Refused(REPOSITORY_UNKNOWN);
    }
    if matched
        .iter()
        .any(|remote| InstallScope::from_remote_options(&remote.options) == InstallScope::User)
    {
        return Authorization::Allowed(MutationScope::USER);
    }
    if matched
        .iter()
        .all(|remote| InstallScope::from_remote_options(&remote.options).is_system_managed())
    {
        return Authorization::Refused(SYSTEM_REMOTE_ONLY);
    }
    // Matched, but flatpak reported a scope this app could not read for any of
    // them. Still not a scope known to succeed.
    Authorization::Refused(REPOSITORY_UNKNOWN)
}

impl InstallScope {
    /// Parse a `flatpak list --columns=installation` cell.
    ///
    /// flatpak prints the literal `user` for the per-user installation,
    /// `system` for the default system-wide one, and the installation's own
    /// name for an extra one. An empty cell means flatpak reported nothing,
    /// which is [`InstallScope::Unknown`] rather than a guess.
    #[must_use]
    pub(crate) fn from_installation_cell(cell: &str) -> Self {
        let trimmed = cell.trim();
        match trimmed.to_ascii_lowercase().as_str() {
            "" => Self::Unknown,
            "user" => Self::User,
            "system" => Self::System,
            // Keep the original spelling: this becomes `--installation=NAME`.
            _ => Self::Named(trimmed.to_owned()),
        }
    }

    /// Parse the scope out of a `flatpak remotes --columns=options` cell.
    ///
    /// The cell is a comma-separated flag list carrying `user` or `system`
    /// among other flags. This is the reading `ui::library::remote_scope` has
    /// always done; it lives here now so a repository row and an installed app
    /// row share one vocabulary. A remote cell never names an installation, so
    /// this never returns [`InstallScope::Named`].
    #[must_use]
    pub(crate) fn from_remote_options(options: &str) -> Self {
        let lowered = options.to_ascii_lowercase();
        let has = |flag: &str| lowered.split(',').any(|field| field.trim() == flag);
        if has("user") {
            Self::User
        } else if has("system") {
            Self::System
        } else {
            Self::Unknown
        }
    }

    /// A user-facing installation name, including runtime-defined installations.
    #[must_use]
    pub(crate) fn label(&self) -> String {
        match self {
            Self::User => "Your user installation".to_owned(),
            Self::System => "The system-wide installation".to_owned(),
            Self::Named(name) => format!("The \"{name}\" installation"),
            Self::Unknown => "An installation whose scope is unreadable".to_owned(),
        }
    }

    /// Whether this scope is a system-wide installation (default or named),
    /// as opposed to the user's own. Only a named one is refused with
    /// [`SYSTEM_MANAGED`]; see [`mutation`](Self::mutation).
    #[must_use]
    pub(crate) fn is_system_managed(&self) -> bool {
        matches!(self, Self::System | Self::Named(_))
    }

    // Resolve installed-row mutations from their recorded scope; never guess a user installation.
    #[must_use]
    pub(crate) fn mutation(&self) -> Authorization {
        match self {
            Self::User => Authorization::Allowed(MutationScope::USER),
            // Preinstalled apps (Zen) live system-wide; Flatpak's polkit policy lets the
            // desktop user update and remove them.
            Self::System => Authorization::Allowed(MutationScope::SYSTEM),
            Self::Named(_) => Authorization::Refused(SYSTEM_MANAGED),
            Self::Unknown => Authorization::Refused(SCOPE_UNKNOWN),
        }
    }
}

/// Authorize reinstalling a removed app from the source recorded at removal.
/// Only the same-named user remote with the same URL qualifies; a removed,
/// renamed, re-pointed or system-only remote refuses as an unknown repository.
#[must_use]
pub(crate) fn removed_origin_authorization(
    origin: &RemovedOrigin,
    load: &RemoteLoad,
    remotes: &[RemoteInfo],
) -> Authorization {
    match load {
        RemoteLoad::Loading => return Authorization::Refused(REPOSITORIES_LOADING),
        RemoteLoad::Failed(_) => return Authorization::Refused(REPOSITORIES_UNAVAILABLE),
        RemoteLoad::Loaded => {}
    }
    match origin.current_remote(remotes) {
        Some(_) if origin.scope == InstallScope::User => {
            Authorization::Allowed(MutationScope::USER)
        }
        _ => Authorization::Refused(REPOSITORY_UNKNOWN),
    }
}
