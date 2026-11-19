//! Update reads across scopes.

use super::*;

// One scope failing does not hide the other scope's updates, and the same id
// installed in both scopes is reported once.
//
#[test]
fn a_single_scope_update_failure_still_reports_the_other_scopes_updates() {
    let executor = ScopeRecordingExecutor::new(&[
        (
            "remote-ls --updates --columns=application --user",
            Err::<&str, &str>("flatpak exited with status 1: no user remotes"),
        ),
        (
            "remote-ls --updates --columns=application --system",
            Ok("org.gimp.GIMP\n"),
        ),
    ]);
    let scan = scan_updates(&executor);
    let partial = &scan.updates;
    assert_eq!(*partial, vec![AvailableUpdate::system("org.gimp.GIMP")]);
    // Partial updates must remain accompanied by the failed scope report.
    assert!(
        !scan.all_scopes_completed(),
        "a scope failed, so this scan is not complete"
    );
    let failed = scan.failed_reports();
    assert_eq!(failed.len(), 1, "exactly one scope failed");
    assert_eq!(failed[0].scope, InstallScope::User);
    assert_eq!(
        scan.completed_scopes(),
        vec![InstallScope::System],
        "and the scope that answered is named too, so a surface can say what \
         IS known"
    );
    assert!(
        !partial[0].reported_by_user_scope(),
        "the scope that FAILED reported nothing, so nothing can be retargeted \
         at it — a partial read narrows what is actionable, never widens it"
    );
    assert_eq!(
        partial[0].authorization(),
        Authorization::Refused(SYSTEM_MANAGED)
    );

    // The dual-installed case: one row in each scope, one badge entry.
    let both = ScopeRecordingExecutor::new(&[
        (
            "remote-ls --updates --columns=application --user",
            Ok::<&str, &str>("org.gimp.GIMP\n"),
        ),
        (
            "remote-ls --updates --columns=application --system",
            Ok("org.gimp.GIMP\norg.gnome.Weather\n"),
        ),
    ]);
    let merged_scan = scan_updates(&both);
    assert!(merged_scan.all_scopes_completed());
    let merged = &merged_scan.updates;
    assert_eq!(
        merged.iter().map(|u| u.app_id.as_str()).collect::<Vec<_>>(),
        ["org.gimp.GIMP", "org.gnome.Weather"],
        "an app reported by both scopes must be ONE entry, user scope first"
    );
    // Deduplicate application IDs while preserving the installation authority of each update.
    assert_eq!(
        merged[0].scopes,
        vec![InstallScope::User, InstallScope::System],
        "GIMP was reported by both, in query order"
    );
    assert_eq!(
        merged[1].scopes,
        vec![InstallScope::System],
        "Weather was reported only by the system query"
    );
    assert_eq!(
        merged[0].authorization(),
        Authorization::Allowed(MutationScope::USER)
    );
    assert_eq!(
        merged[1].authorization(),
        Authorization::Refused(SYSTEM_MANAGED),
        "a system-only update is never resolved to --user"
    );
}

// Exercise complete and partial update scans through the injected executor.
pub(super) fn scan_updates(executor: &dyn FlatpakExecutor) -> crate::services::UpdateScan {
    list_updates_with_executor(false, executor, &OperationCancellation::default())
}
