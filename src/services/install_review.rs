//! Native permission worker protocol; no Flatpak dependency at frontend startup.
use super::{OperationCancellation, process};
use enzui_services::process_transport::StdinPolicy;
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReviewedInstall {
    pub app_id: String,
    pub update: bool,
    pub remote: String,
    /// The ref the review resolved; confirmation must resolve the same one.
    pub target: RefTarget,
    pub token: String,
    pub changes: Vec<Value>,
    /// Update an app in the system-wide installation.
    pub system: bool,
    /// No permission service on this system (any desktop but enzimOS):
    /// Flatpak installs or updates the app itself, from `remote`, with the
    /// access the app asks for. `token` and `changes` are empty.
    pub direct: bool,
}

/// The enzimOS permission worker that reviews and applies installs.
const PERMISSION_SERVICE: &str = "/usr/bin/enzimos-app-permissions";

/// Whether this system has the permission service. Without it Store installs
/// and updates through Flatpak directly.
#[cfg(not(test))]
pub(crate) fn service_available() -> bool {
    process::resolve_authority(Path::new(PERMISSION_SERVICE)).is_ok()
}

/// The branch and arch an install asks for. Empty means the remote's default
/// branch and this machine's arch; a reinstall names what was removed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RefTarget {
    pub branch: Option<String>,
    pub arch: Option<String>,
}

impl RefTarget {
    /// The recorded deployment of a removed app; unrecorded parts stay default.
    pub(crate) fn recorded(branch: &str, arch: &str) -> Self {
        let part = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        Self {
            branch: part(branch),
            arch: part(arch),
        }
    }

    fn extend(&self, body: &mut Value) {
        if let Some(branch) = &self.branch {
            body["branch"] = json!(branch);
        }
        if let Some(arch) = &self.arch {
            body["arch"] = json!(arch);
        }
    }
}

/// Plain-language text for a worker's fixed refusal reason. Unknown or absent
/// reasons keep the generic text; worker output is never shown verbatim.
pub(crate) fn reason_message(reason: Option<&str>) -> Option<&'static str> {
    Some(match reason? {
        "runtime-unavailable" => {
            "The app needs a runtime that no configured repository provides. Installation has not started."
        }
        "app-not-found" => {
            "The repository no longer offers this app. Installation has not started."
        }
        "remote-not-found" => {
            "The repository is no longer configured. Installation has not started."
        }
        "repository-unreachable" => {
            "The repository could not be reached. Check the connection and try again."
        }
        "untrusted-source" => {
            "The repository's signature could not be verified. Installation has not started."
        }
        "needs-newer-flatpak" => {
            "This app needs a newer version of Flatpak. Installation has not started."
        }
        "invalid-data" => "The repository returned invalid app data. Installation has not started.",
        "out-of-space" => {
            "There is not enough disk space for this app. Installation has not started."
        }
        "not-authorized" => {
            "The repository refused access to this app. Installation has not started."
        }
        "timed-out" => "The permission review took too long. Check the repository and try again.",
        "app-not-resolved" => {
            "The repository did not resolve this app. Installation has not started."
        }
        "installed-source-changed" => "The app's installed source changed. Review it again.",
        "not-installed" => "The app is no longer installed.",
        "review-stale" => "The app or its permissions changed during review. Review it again.",
        "review-incomplete" => {
            "Flatpak could not prepare this installation. Installation has not started."
        }
        _ => return None,
    })
}

pub(crate) fn request(
    body: Value,
    timeout: Duration,
    cancellation: &OperationCancellation,
) -> Result<Value, String> {
    request_with_program(Path::new(PERMISSION_SERVICE), body, timeout, cancellation)
}

fn request_with_program(
    program: &Path,
    body: Value,
    timeout: Duration,
    cancellation: &OperationCancellation,
) -> Result<Value, String> {
    let executable = process::resolve_authority(program).map_err(|_| {
        "Permission service is unavailable. Installation has not started.".to_owned()
    })?;
    let raw = serde_json::to_vec(&body).map_err(|_| "Invalid permission request".to_owned())?;
    if raw.len() > 1024 * 1024 {
        return Err("Permission request is too large".into());
    }
    let process_request = process::build_request(
        executable,
        &[],
        StdinPolicy::Bytes(raw),
        1024 * 1024,
        65536,
        timeout,
        process::FLATPAK_ENV_ALLOWLIST,
    )
    .map_err(|_| "Permission service could not start".to_owned())?;
    let output = process::run_owned(process_request, cancellation)
        .map_err(|_| "Permission service was interrupted or did not respond".to_owned())?;
    let reply: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Invalid permission service response".to_owned())?;
    if !output.status.success() || reply.get("ok") != Some(&Value::Bool(true)) {
        return Err(reason_message(reply["reason"].as_str()).map_or_else(
            || "Permission review failed. Check the repository and try again; installation has not been verified.".into(), str::to_owned));
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| "Incomplete permission service response".into())
}

pub(crate) fn prepare(
    app_id: &str,
    remote: &str,
    target: &RefTarget,
    update: bool,
    system: bool,
    cancellation: &OperationCancellation,
) -> Result<Value, String> {
    prepare_with_program(
        Path::new(PERMISSION_SERVICE),
        app_id,
        remote,
        target,
        update,
        system,
        cancellation,
    )
}

fn prepare_with_program(
    program: &Path,
    app_id: &str,
    remote: &str,
    target: &RefTarget,
    update: bool,
    system: bool,
    cancellation: &OperationCancellation,
) -> Result<Value, String> {
    crate::flatpak_args::validate_app_id(app_id)?;
    let mut body = json!({"schema_version":1,"method":if update {"update.review"} else {"install.review"},"app_id":app_id,"remote":remote});
    if system && update {
        body["installation"] = json!("system");
    }
    if !update {
        target.extend(&mut body);
    }
    let reply = request_with_program(program, body, Duration::from_secs(125), cancellation)?;
    if reply["status"] != "review-required" {
        return Err(reason_message(reply["reason"].as_str())
            .unwrap_or("Installation did not return a permission review")
            .into());
    }
    let plan = reply["plan"].clone();
    validate_plan(&plan, app_id, remote)?;
    if (plan["action"] == "update") != update {
        return Err("Permission review action changed".into());
    }
    Ok(plan)
}

pub(crate) fn validate_plan(plan: &Value, app_id: &str, remote: &str) -> Result<(), String> {
    let token = plan["review_token"]
        .as_str()
        .ok_or("Permission review has no identity")?;
    if token.len() != 64
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || plan["schema_version"] != 1
        || plan["remote"]["name"] != remote
    {
        return Err("Invalid permission review identity".into());
    }
    let operations = plan["operations"]
        .as_array()
        .ok_or("Permission review has no resolved packages")?;
    let update = plan["action"] == "update";
    let installed_ref = plan["installed"]["ref"]
        .as_str()
        .is_some_and(|reference| reference.starts_with(&format!("app/{app_id}/")));
    if operations.len() > 128
        || (!update && operations.is_empty())
        || (!installed_ref
            && !operations.iter().any(|op| {
                op["ref"]
                    .as_str()
                    .is_some_and(|reference| reference.starts_with(&format!("app/{app_id}/")))
            }))
    {
        return Err("Permission review does not contain the requested app".into());
    }
    let old_commit_valid = plan["installed"]["commit"].as_str().is_some_and(|commit| {
        commit.len() == 64
            && commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if update
        && (!installed_ref
            || !old_commit_valid
            || !plan["requires_consent"].is_boolean()
            || !plan["permission_additions"].is_array())
    {
        return Err("Incomplete update permission comparison".into());
    }
    for operation in operations {
        let commit = operation["commit"]
            .as_str()
            .ok_or("Resolved package has no commit")?;
        if commit.len() != 64
            || !commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || !operation["metadata"].is_object()
        {
            return Err("Incomplete resolved package permissions".into());
        }
    }
    Ok(())
}

pub(crate) fn install(
    review: &ReviewedInstall,
    cancellation: &OperationCancellation,
    sink: super::ProgressSink<'_>,
) -> Result<(), String> {
    use enzui_services::process_transport::OutputStream;
    let mut body = json!({"schema_version":1,"method":if review.update {"update.confirm"} else {"install.confirm"},"stream":true,"app_id":review.app_id,"remote":review.remote,"review_token":review.token,"changes":review.changes});
    if !review.update {
        review.target.extend(&mut body);
    }
    if review.system && review.update {
        body["installation"] = json!("system");
    }
    let executable = process::resolve_authority(Path::new(PERMISSION_SERVICE))
        .map_err(|_| "Permission service unavailable")?;
    let request = process::build_request(
        executable,
        &[],
        StdinPolicy::Bytes(serde_json::to_vec(&body).map_err(|_| "Invalid install request")?),
        1024 * 1024,
        65536,
        Duration::from_secs(6 * 3600 + 60),
        process::FLATPAK_ENV_ALLOWLIST,
    )
    .map_err(|_| "Installation worker could not start")?;
    let mut line = Vec::new();
    let mut oversized = false;
    let output = process::run_owned_streaming(request, cancellation, |chunk| {
        if chunk.stream != OutputStream::Stdout {
            return;
        }
        for byte in chunk.bytes {
            if byte == b'\n' {
                if !oversized
                    && let Ok(event) = serde_json::from_slice::<Value>(&line)
                    && event["event"]
                        == if review.update {
                            "update.progress"
                        } else {
                            "install.progress"
                        }
                    && let (Some(percent), Some(package), Some(packages)) = (
                        event["operation_percent"].as_u64(),
                        event["package"].as_u64(),
                        event["packages"].as_u64(),
                    )
                    && percent <= 100
                    && packages > 0
                    && packages <= 128
                    && package > 0
                    && package <= packages
                {
                    sink(super::ProgressUpdate {
                        app_id: review.app_id.clone(),
                        progress: (((package - 1) as f32 + percent as f32 / 100.0)
                            / packages as f32)
                            .min(0.99),
                        message: format!("Installing package {package} of {packages}"),
                    });
                }
                line.clear();
                oversized = false;
            } else if line.len() < 8192 {
                line.push(byte);
            } else {
                oversized = true;
            }
        }
    })
    .map_err(|_| "Installation interrupted; check the app's installed state before retrying")?;
    let reply: Value = output
        .stdout
        .split(|byte| *byte == b'\n')
        .rfind(|line| !line.is_empty())
        .ok_or("Installation returned no result")
        .and_then(|line| serde_json::from_slice(line).map_err(|_| "Invalid installation result"))?;
    if !output.status.success()
        || reply["ok"] != true
        || reply["result"]["status"]
            != if review.update {
                "updated"
            } else {
                "installed"
            }
    {
        return Err("Installation could not be verified. Any saved permission restrictions remain in place. Review the app before retrying.".into());
    }
    Ok(())
}
