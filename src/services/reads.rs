pub(crate) fn list_installed_cancellable(
    cancellation: OperationCancellation,
) -> Result<Vec<AppEntry>, String> {
    let executor = ProcessFlatpakExecutor::cancellable(cancellation.clone());
    let mut entries = list_installed_with_executor(fixture_enabled(), &executor)?;
    local_icons::LocalIcons::from_env().hydrate(&mut entries, &|| cancellation.is_cancelled());
    Ok(entries)
}

/// Installed-app read with the fixture flag supplied directly.
pub(crate) fn list_installed_with_fixture(fixture: bool) -> Result<Vec<AppEntry>, String> {
    list_installed_with_executor(fixture, &ProcessFlatpakExecutor::default())
}

// Requested columns and parser indices must change together; missing installation data stays Unknown.
const INSTALLED_COLUMNS_ARG: &str = "--columns=application,name,version,branch,origin,size,installation,description,arch:f,active:f";

fn list_installed_with_executor(
    fixture: bool,
    executor: &dyn FlatpakExecutor,
) -> Result<Vec<AppEntry>, String> {
    if fixture {
        return Ok(fixture_installed());
    }
    let raw = run_flatpak(executor, &["list", "--app", INSTALLED_COLUMNS_ARG])?;
    let mut apps = Vec::new();
    for cols in parse_columns(&raw) {
        if cols.len() < 5 {
            continue;
        }
        let id = cols[0].clone();
        if crate::flatpak_args::validate_app_id(&id).is_err() {
            continue;
        }
        let name = if cols[1].is_empty() {
            id.clone()
        } else {
            cols[1].clone()
        };
        apps.push(AppEntry {
            id,
            name,
            version: cols.get(2).cloned().unwrap_or_default(),
            branch: cols.get(3).cloned().unwrap_or_default(),
            origin: cols.get(4).cloned().unwrap_or_default(),
            size: cols.get(5).cloned().unwrap_or_default(),
            installed: true,
            deployed_arch: cols.get(8).cloned().unwrap_or_default(),
            deployed_commit: cols.get(9).cloned().unwrap_or_default(),
            summary: cols
                .get(7)
                .map(|value| {
                    value
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(512)
                        .collect()
                })
                .unwrap_or_default(),
            // Missing installation cells must not imply user scope.
            install_scope: InstallScope::from_installation_cell(
                cols.get(6).map(String::as_str).unwrap_or_default(),
            ),
            ..Default::default()
        });
    }
    Ok(apps)
}

// `flatpak remotes --columns=name,title,url,options`.
pub(crate) fn list_remotes_cancellable(
    cancellation: OperationCancellation,
) -> Result<Vec<RemoteInfo>, String> {
    let executor = ProcessFlatpakExecutor::cancellable(cancellation);
    list_remotes_with_executor(fixture_enabled(), &executor)
}

/// Remote-list read with the fixture flag supplied directly.
pub(crate) fn list_remotes_with_fixture(fixture: bool) -> Result<Vec<RemoteInfo>, String> {
    list_remotes_with_executor(fixture, &ProcessFlatpakExecutor::default())
}

fn list_remotes_with_executor(
    fixture: bool,
    executor: &dyn FlatpakExecutor,
) -> Result<Vec<RemoteInfo>, String> {
    if fixture {
        return Ok(fixture_remotes());
    }
    let raw = run_flatpak(executor, &["remotes", "--columns=name,title,url,options"])?;
    let mut remotes = Vec::new();
    for cols in parse_columns(&raw) {
        if cols.is_empty() {
            continue;
        }
        let name = cols.first().cloned().unwrap_or_default();
        if crate::flatpak_args::validate_remote_name(&name).is_err() {
            continue;
        }
        remotes.push(RemoteInfo {
            name,
            title: cols.get(1).cloned().unwrap_or_default(),
            url: cols.get(2).cloned().unwrap_or_default(),
            options: cols.get(3).cloned().unwrap_or_default(),
        });
    }
    Ok(remotes)
}

/// An explicitly queried installation and its authority to refresh appstream metadata.
#[derive(Clone, Copy, Debug)]
struct UpdateReadScope {
    /// The `flatpak` option selecting the installation.
    option: &'static str,
    /// Whether `flatpak update --appstream <option>` is an unprivileged write
    /// this build may perform first.
    may_refresh_appstream: bool,
}

/// Query both installations unconditionally; an empty scope result does not describe another scope.
impl UpdateReadScope {
    // The installation this entry selects, as the value an
    // [`AvailableUpdate`] records.
    //
    // Derived from `option` rather than stored beside it so the two cannot
    // disagree: the flag that was sent to flatpak *is* the scope that
    // answered.
    fn installation(self) -> InstallScope {
        match self.option {
            "--user" => InstallScope::User,
            "--system" => InstallScope::System,
            named => InstallScope::Named(named.trim_start_matches("--installation=").to_owned()),
        }
    }
}

const UPDATE_READ_SCOPES: [UpdateReadScope; 2] = [
    UpdateReadScope {
        option: "--user",
        may_refresh_appstream: true,
    },
    UpdateReadScope {
        // `flatpak update --appstream --system` writes `/var/lib/flatpak`
        // through the privileged system helper, so this build must not
        // attempt it silently at boot. The system scope is read against the
        // appstream data that is already on disk; `remote-ls` itself is an
        // unprivileged read.
        option: "--system",
        may_refresh_appstream: false,
    },
];
