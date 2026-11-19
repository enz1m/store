// Launch is a handoff to an independent user service, not an installation worker.
pub(crate) fn launch_installed(
    app_id: &str,
    scope: &InstallScope,
    cancellation: &OperationCancellation,
) -> Result<(), String> {
    launch_installed_with(
        Path::new("/usr/bin/systemd-run"),
        app_id,
        scope,
        cancellation,
    )
    .map(|_| ())
}
pub(crate) fn launch_installed_with(
    launcher: &Path,
    app_id: &str,
    scope: &InstallScope,
    cancellation: &OperationCancellation,
) -> Result<String, String> {
    crate::flatpak_args::validate_app_id(app_id)?;
    let scope = match scope {
        InstallScope::User => "--user".to_owned(),
        InstallScope::System => "--system".to_owned(),
        InstallScope::Named(name) => {
            crate::flatpak_args::validate_remote_name(name)?;
            format!("--installation={name}")
        }
        InstallScope::Unknown => {
            return Err("The app's installation is unknown. Refresh and try again.".into());
        }
    };
    let mut args: Vec<String> = [
        "--user",
        "--collect",
        "--quiet",
        "--service-type=exec",
        "--expand-environment=no",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    // Preserve the selected desktop when another session updates the user manager's environment.
    for key in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_TYPE",
    ] {
        if let Ok(value) = std::env::var(key) {
            args.push(format!("--setenv={key}={value}"));
        }
    }
    args.extend([
        "--".into(),
        "/usr/bin/flatpak".into(),
        "run".into(),
        scope,
        "--".into(),
        app_id.into(),
    ]);
    let failure =
        || "Could not open this app. Refresh the installed apps and try again.".to_owned();
    let executable = process::resolve_authority(launcher).map_err(|_| failure())?;
    let request = process::build_request(
        executable,
        &args,
        process_transport::StdinPolicy::Null,
        65536,
        8192,
        Duration::from_secs(15),
        process::FLATPAK_ENV_ALLOWLIST,
    )
    .map_err(|_| failure())?;
    let output = process::run_owned(request, cancellation).map_err(|_| failure())?;
    if !output.status.success() {
        return Err(failure());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
