// Remote add, replace, and removal.

/// Names owned by read-only installations must not be duplicated in user scope.
pub(crate) const REMOTE_NAME_TAKEN_BY_SYSTEM: Refusal = Refusal {
    label: "Name used system-wide",
    reason: "A system-wide repository already uses this name. Store can only \
             change your user installation, and a second repository with the \
             same name would make installs ambiguous — choose another name.",
};

/// Distinguish an already-correct repository from a name conflict using authoritative configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RemoteAddPlan {
    /// No repository of that name in the target installation: add it.
    Add,
    /// Already configured with exactly this canonical URL, in the target
    /// installation. An honest idempotent success: the user asked for a state
    /// that already holds, so it is reported as done **without running any
    /// child process at all**.
    AlreadyConfigured,
    /// Same name, different canonical URL, in the installation this build can
    /// write. A conflict the user must resolve explicitly; the form is
    /// retained for Replace / Rename / Cancel.
    UrlConflict {
        /// The canonical URL currently configured, for the disclosure.
        existing_url: String,
    },
    /// The name is spoken for by an installation this build cannot write.
    ScopeConflict(Refusal),
}

/// Resolve an authorized scope, refusing system-name conflicts before comparing URLs.
#[must_use]
pub(crate) fn plan_remote_add(
    name: &str,
    url: &str,
    remotes: &[RemoteInfo],
    scope: MutationScope,
) -> RemoteAddPlan {
    let target = scope.installation();
    let mut in_target: Option<&RemoteInfo> = None;
    for remote in remotes.iter().filter(|remote| remote.name == name) {
        let remote_scope = InstallScope::from_remote_options(&remote.options);
        if remote_scope == target {
            in_target = Some(remote);
        } else if remote_scope.is_system_managed() {
            return RemoteAddPlan::ScopeConflict(REMOTE_NAME_TAKEN_BY_SYSTEM);
        } else {
            // The row exists but its installation is unreadable, so this build
            // cannot prove the mutation would land where it intends. Absence
            // of evidence is not evidence of the user installation — the same
            // rule `install_authorization` follows for an unresolvable origin.
            return RemoteAddPlan::ScopeConflict(SCOPE_UNKNOWN);
        }
    }
    match in_target {
        None => RemoteAddPlan::Add,
        Some(existing) => {
            if normalise_repository_url(&existing.url) == normalise_repository_url(url) {
                RemoteAddPlan::AlreadyConfigured
            } else {
                RemoteAddPlan::UrlConflict {
                    existing_url: existing.url.clone(),
                }
            }
        }
    }
}

// The repository read and add decision share one task to avoid stale plans.
pub(crate) fn plan_remote_add_with(
    executor: &dyn FlatpakExecutor,
    name: &str,
    url: &str,
    scope: MutationScope,
) -> Result<RemoteAddPlan, String> {
    if !crate::model::is_valid_remote_url(url) {
        return Err(invalid_repository_url(ADD_ACTION, name));
    }
    if let Err(diagnostic) = crate::flatpak_args::validate_remote_name(name) {
        return Err(repository_failure(ADD_ACTION, name, &diagnostic));
    }
    let remotes = list_remotes_with_executor(false, executor)?;
    Ok(plan_remote_add(name, url, &remotes, scope))
}

/// The production entry point for the pre-mutation read.
pub(crate) fn plan_remote_add_request(name: &str, url: &str) -> Result<RemoteAddPlan, String> {
    plan_remote_add_with(
        &ProcessFlatpakExecutor::default(),
        name,
        url,
        MutationScope::USER,
    )
}

pub(crate) fn add_remote(name: &str, url: &str) -> Result<(), String> {
    add_remote_with(
        &ProcessFlatpakExecutor::default(),
        name,
        url,
        MutationScope::USER,
    )
}

// Do not mask an unexpected existing name with --if-not-exists.
pub(crate) fn add_remote_with(
    executor: &dyn FlatpakExecutor,
    name: &str,
    url: &str,
    scope: MutationScope,
) -> Result<(), String> {
    if !crate::model::is_valid_remote_url(url) {
        return Err(invalid_repository_url(ADD_ACTION, name));
    }
    let mut args = crate::flatpak_args::FlatpakArgs::new("remote-add");
    args.options([scope.option()]);
    if let Err(diagnostic) = args.remote(name) {
        return Err(repository_failure(ADD_ACTION, name, &diagnostic));
    }
    args.literal_operand(url);
    executor
        .execute(&args.finish())
        .map(|_| ())
        // The backend diagnostic verbatim, attributed to the repository and
        // the action the user pressed — the retained form offers two.
        .map_err(|diagnostic| repository_failure(ADD_ACTION, name, &diagnostic))
}

// Replace in place, verify via readback, and restore the prior URL on mismatch.
pub(crate) fn replace_remote_url_with(
    executor: &dyn FlatpakExecutor,
    name: &str,
    new_url: &str,
    old_url: &str,
    scope: MutationScope,
) -> Result<String, String> {
    if !crate::model::is_valid_remote_url(new_url) {
        return Err(invalid_repository_url(REPLACE_ACTION, name));
    }
    if let Err(diagnostic) = crate::flatpak_args::validate_remote_name(name) {
        return Err(repository_failure(REPLACE_ACTION, name, &diagnostic));
    }
    // A refused modify changed nothing, so there is nothing to restore and NO
    // rollback is issued — a second `remote-modify` here would be a write the
    // user's repository never needed, on a path that has just proved writes
    // are failing. The diagnostic is wrapped, not replaced: the raw backend
    // text alone ("Remote \"ghost\" not found") does not say which action the
    // user pressed, and the retained form offers two.
    if let Err(diagnostic) = modify_remote_url(executor, name, new_url, scope) {
        return Err(repository_failure(REPLACE_ACTION, name, &diagnostic));
    }

    let settled = match read_remote_url(executor, name, scope) {
        Ok(Some(settled)) => settled,
        Ok(None) => {
            return Err(restore_failed_replacement(
                executor,
                name,
                old_url,
                scope,
                "it is no longer configured after the change",
            ));
        }
        Err(error) => {
            return Err(restore_failed_replacement(
                executor,
                name,
                old_url,
                scope,
                &format!("the change could not be confirmed: {error}"),
            ));
        }
    };
    if normalise_repository_url(&settled) != normalise_repository_url(new_url) {
        return Err(restore_failed_replacement(
            executor,
            name,
            old_url,
            scope,
            &format!("it still points at {}", redact_url_credentials(&settled)),
        ));
    }
    Ok(settled)
}

/// The production entry point for a confirmed Replace.
pub(crate) fn replace_remote_url(
    name: &str,
    new_url: &str,
    old_url: &str,
) -> Result<String, String> {
    replace_remote_url_with(
        &ProcessFlatpakExecutor::default(),
        name,
        new_url,
        old_url,
        MutationScope::USER,
    )
}

// `flatpak remote-modify <scope> --url=<url> -- <name>`.
//
// The URL rides in an option, so it is validated by the caller before it gets
// here and the operand boundary still protects the name.
fn modify_remote_url(
    executor: &dyn FlatpakExecutor,
    name: &str,
    url: &str,
    scope: MutationScope,
) -> Result<(), String> {
    debug_assert!(
        crate::model::is_valid_remote_url(url),
        "a URL reaches an option only after validation"
    );
    let mut args = crate::flatpak_args::FlatpakArgs::new("remote-modify");
    args.options([scope.option()]);
    args.option_with_value("--url", url);
    args.remote(name)?;
    executor.execute(&args.finish()).map(|_| ())
}

// The URL currently configured for `name` in `scope`, or `None` when no such
// repository is configured there.
fn read_remote_url(
    executor: &dyn FlatpakExecutor,
    name: &str,
    scope: MutationScope,
) -> Result<Option<String>, String> {
    let target = scope.installation();
    Ok(list_remotes_with_executor(false, executor)?
        .into_iter()
        .find(|remote| {
            remote.name == name && InstallScope::from_remote_options(&remote.options) == target
        })
        .map(|remote| remote.url))
}

// Put `old_url` back after a failed replacement and describe the outcome.
//
// Always returns an error message: this is only reached when the replacement
// did not settle, so the caller has nothing to report as success.
fn restore_failed_replacement(
    executor: &dyn FlatpakExecutor,
    name: &str,
    old_url: &str,
    scope: MutationScope,
    failure: &str,
) -> String {
    let shown = redact_url_credentials(old_url);
    if !crate::model::is_valid_remote_url(old_url) {
        return repository_failure(
            REPLACE_ACTION,
            name,
            &format!(
                "{failure}. The previous URL could not be restored because it \
                 is not a valid repository URL."
            ),
        );
    }
    match modify_remote_url(executor, name, old_url, scope) {
        Ok(()) => repository_failure(
            REPLACE_ACTION,
            name,
            &format!("{failure}. It has been restored to {shown}."),
        ),
        Err(restore) => repository_failure(
            REPLACE_ACTION,
            name,
            &format!("{failure}. Restoring {shown} also failed: {restore}"),
        ),
    }
}

pub(crate) fn remove_remote(name: &str) -> Result<(), String> {
    remove_remote_with(&ProcessFlatpakExecutor::default(), name)
}

pub(crate) fn remove_remote_with(executor: &dyn FlatpakExecutor, name: &str) -> Result<(), String> {
    let mut args = crate::flatpak_args::FlatpakArgs::new("remote-delete");
    args.options(["--user", "--force"]);
    args.remote(name)?;
    executor.execute(&args.finish()).map(|_| ())
}
