//! Shared checked permission authority for installed app policies.
use super::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Default)]
pub(crate) struct NativePermissionRunner {
    snapshots: Mutex<HashMap<String, Value>>,
}

impl NativePermissionRunner {
    fn write(
        &self,
        app_id: &str,
        mut changes: Vec<Value>,
        reset: bool,
        cancellation: OperationCancellation,
    ) -> Result<(), String> {
        // Consume the shown snapshot before a write. Concurrent clicks cannot
        // silently rebase onto a policy the user has not seen.
        let snapshot = self
            .snapshots
            .lock()
            .map_err(|_| "Permission state unavailable")?
            .remove(app_id)
            .ok_or("Permission policy must be reviewed again")?;
        for change in &mut changes {
            if change["category"] == "filesystems"
                && change["allow"] == true
                && let Some(path) = change["value"]
                    .as_str()
                    .filter(|path| matches!(*path, "home" | "host"))
            {
                change["value"] = Value::String(restored_filesystem_grant(&snapshot, path));
            }
        }
        let token = snapshot["review_token"]
            .as_str()
            .ok_or("Invalid policy identity")?;
        let result = install_review::request(
            json!({"schema_version":1,"method":"apply",
            "subject":snapshot["subject"],"review_token":token,"changes":changes,"reset":reset}),
            Duration::from_secs(20),
            &cancellation,
        )?;
        if result["status"] == "applied" || result["status"] == "unchanged" {
            return Ok(());
        }
        Err("Permission write readback unavailable; review current policy before retrying".into())
    }
}

fn restored_filesystem_grant(snapshot: &Value, path: &str) -> String {
    for context in ["flatpak_computed_context", "metadata"] {
        if let Some(values) = snapshot[context]["Context"]["filesystems"].as_str()
            && let Some(value) = values.split(';').rev().find(|value| {
                *value == path
                    || [":ro", ":rw", ":create"]
                        .iter()
                        .any(|mode| value.strip_suffix(mode) == Some(path))
            })
        {
            return value.to_owned();
        }
    }
    // An undeclared filesystem grant is conservative, never silently widened
    // to read/write by a common boolean control. Detailed mode edits belong
    // in the shared permission editor.
    format!("{path}:ro")
}

impl PermissionOverrideRunner for NativePermissionRunner {
    fn read(
        &self,
        app_id: &str,
        scope: &str,
        cancellation: OperationCancellation,
    ) -> PermissionTogglesOutcome {
        let result = install_review::request(
            json!({"schema_version":1,"method":"inspect",
            "subject":{"app_id":app_id,"scope":scope}}),
            Duration::from_secs(20),
            &cancellation,
        );
        let Ok(snapshot) = result else {
            return PermissionTogglesOutcome::Unavailable;
        };
        let Ok(toggles) = snapshot_toggles(&snapshot) else {
            return PermissionTogglesOutcome::Unavailable;
        };
        let Ok(mut cache) = self.snapshots.lock() else {
            return PermissionTogglesOutcome::Unavailable;
        };
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(app_id.into(), snapshot.clone());
        PermissionTogglesOutcome::ReadyPolicy {
            toggles,
            snapshot: Box::new(snapshot),
        }
    }
    fn apply(&self, app_id: &str, key: &str, enable: bool) -> Result<(), String> {
        self.apply_cancellable(app_id, key, enable, OperationCancellation::default())
    }
    fn apply_cancellable(
        &self,
        app_id: &str,
        key: &str,
        enable: bool,
        cancellation: OperationCancellation,
    ) -> Result<(), String> {
        let (category, value) = match key {
            "network" => ("shared", "network"),
            "home" => ("filesystems", "home"),
            "host" => ("filesystems", "host"),
            "wayland" => ("sockets", "wayland"),
            "x11" => ("sockets", "x11"),
            "pulseaudio" => ("sockets", "pulseaudio"),
            "all" => ("devices", "all"),
            _ => return Err("Unknown permission control".into()),
        };
        let mut changes = vec![json!({"category":category,"value":value,"allow":enable})];
        if key == "x11" && !enable {
            changes.push(json!({"category":"sockets","value":"fallback-x11","allow":false}));
        }
        self.write(app_id, changes, false, cancellation)
    }
    fn reset(&self, app_id: &str) -> Result<(), String> {
        self.reset_cancellable(app_id, OperationCancellation::default())
    }
    fn reset_cancellable(
        &self,
        app_id: &str,
        cancellation: OperationCancellation,
    ) -> Result<(), String> {
        self.write(app_id, vec![], true, cancellation)
    }
    fn checked_policy(&self) -> bool {
        true
    }
}

fn snapshot_toggles(snapshot: &Value) -> Result<Vec<PermissionToggle>, String> {
    if snapshot["schema_version"] != 1
        || !snapshot["flatpak_computed_context"].is_object()
        || !snapshot["metadata"].is_object()
        || snapshot["review_token"].as_str().is_none()
    {
        return Err("Incomplete shared permission snapshot".into());
    }
    fn context(raw: &Value) -> Result<Permissions, String> {
        let groups = raw.as_object().ok_or("Invalid context")?;
        let mut keyfile = String::new();
        for (group, values) in groups {
            let values = values.as_object().ok_or("Invalid policy group")?;
            if group.contains(['\r', '\n', ']']) {
                return Err("Invalid group name".into());
            }
            keyfile.push_str(&format!("[{group}]\n"));
            for (key, value) in values {
                let value = value.as_str().ok_or("Invalid policy value")?;
                if key.contains(['\r', '\n', '=']) || value.contains(['\r', '\n']) {
                    return Err("Unsupported policy value".into());
                }
                keyfile.push_str(&format!("{key}={value}\n"));
            }
        }
        Ok(parse_metadata(&keyfile))
    }
    let declared = context(&snapshot["metadata"])?;
    let computed = context(&snapshot["flatpak_computed_context"])?;
    Ok(COMMON_TOGGLES
        .iter()
        .map(|(key, label, category)| PermissionToggle {
            key: (*key).into(),
            label: (*label).into(),
            category: (*category).into(),
            app_default: perm_is_set_in(&declared, key),
            enabled: perm_is_set_in(&computed, key),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restoring_common_filesystem_access_preserves_mode_without_widening() {
        let readonly = json!({"metadata":{"Context":{"filesystems":"home:ro;host-os;"}},"flatpak_computed_context":{"Context":{"filesystems":"!home;"}}});
        assert_eq!(restored_filesystem_grant(&readonly, "home"), "home:ro");
        assert_eq!(restored_filesystem_grant(&readonly, "host"), "host:ro");
        let writable = json!({"metadata":{"Context":{"filesystems":"home:ro;"}},"flatpak_computed_context":{"Context":{"filesystems":"home:rw;"}}});
        assert_eq!(restored_filesystem_grant(&writable, "home"), "home:rw");
    }
}
