//! Dual installations: projection, survival notices, and per-copy authorization.

use super::*;

// The **positive control** for the same dual-installed shape: when the user
// query reports it, the update is actionable and resolves to exactly `--user`.
//
// Without this, the test above would pass on a Library route that stopped
// rendering Update buttons at all.
#[test]
fn a_user_reported_update_on_a_dual_installed_app_resolves_to_exactly_user_scope() {
    let mut store = library_store(
        installed_in_both("org.test.Hero", "HeroApp"),
        vec![remote("flathub", "user")],
    );
    store.deliver_known_updates(vec![AvailableUpdate::user("org.test.Hero")]);
    store.refresh_projection();

    let authorization = store.update_authorization("org.test.Hero");
    assert_eq!(authorization, Authorization::Allowed(MutationScope::USER));
    let scope = authorization.scope().expect("allowed");
    assert_eq!(scope.option(), "--user");
    assert_eq!(
        crate::services::mutation_args("update", Some("org.test.Hero"), scope).expect("argv"),
        ["update", "--user", "-y", "--", "org.test.Hero"]
            .map(str::to_owned)
            .to_vec()
    );

    let ns = nodes(&store);
    assert_eq!(
        role_count(&ns, Role::Button, "Update HeroApp"),
        1,
        "labels: {:?}",
        labels(&ns)
    );
    assert_eq!(label_count(&ns, SYSTEM_MANAGED.label), 0);
}

// A failed installation read must not grant mutation authority for that scope.
#[test]
fn a_partial_update_read_cannot_retarget_an_update_at_the_user_installation() {
    let mut store = library_store(
        installed_in_both("org.test.Hero", "HeroApp"),
        vec![remote("flathub", "user")],
    );
    // What the executor produces when `--user` fails and `--system` answers.
    store.deliver_known_updates(vec![AvailableUpdate::system("org.test.Hero")]);
    store.refresh_projection();

    assert_eq!(
        store.update_authorization("org.test.Hero"),
        Authorization::Refused(SYSTEM_MANAGED),
        "a scope that did not answer cannot be the scope an action runs in"
    );
    assert_eq!(
        role_count(&nodes(&store), Role::Button, "Update HeroApp"),
        0
    );

    // Update all must skip it too: it is installed in the user scope, which is
    // exactly the wrong reason to act.
    let _ = store.update(Message::UpdateAllRequested);
    crate::install_consent::approve_fixture(&mut store);
    assert!(
        store.operations.is_empty(),
        "no operation may start for an update no user-scope query reported"
    );
}

// **The reducer contract, across the removal.** Removing the user copy of a
// dual-installed app settles the USER row, leaves the system row installed,
// and the refresh reveals it as the system copy, which Store can now remove
// in the system installation.
//
// The old arm removed the first row carrying the id, which for a
// system-then-user read order deleted the *system* row and left the user copy
// listed — reporting the exact inverse of what happened.
//
#[test]
fn removing_the_user_copy_leaves_the_system_copy_installed_and_removable_in_the_system_installation()
 {
    let mut store = library_store(
        installed_in_both("org.test.Hero", "HeroApp"),
        vec![remote("flathub", "user")],
    );
    assert_eq!(store.installed.len(), 2, "two installations, one app");

    // Before: one row, Remove offered, survival disclosed.
    let before = nodes(&store);
    assert_eq!(role_count(&before, Role::Button, "Remove HeroApp"), 1);
    assert_eq!(
        label_count(&before, crate::services::SYSTEM_COPY_REMAINS.label),
        1
    );

    let _ = store.update(Message::UninstallConfirmRequested(
        "org.test.Hero".into(),
        "HeroApp".into(),
    ));
    confirm_pending_uninstall(&mut store);
    let _ = store.update(Message::OperationEvent(crate::flatpak::OpEvent::Finished {
        id: current_operation_id(&store, "org.test.Hero"),
        terminal: crate::flatpak::OperationTerminal::Succeeded,
    }));

    // After: the USER row is gone, the SYSTEM row is still installed.
    assert_eq!(
        store
            .installed
            .iter()
            .filter(|e| e.id == "org.test.Hero")
            .map(|e| e.install_scope.clone())
            .collect::<Vec<_>>(),
        vec![InstallScope::System],
        "the settled installation is the one that was removed"
    );
    assert!(
        !store
            .recently_uninstalled
            .iter()
            .any(|e| e.id == "org.test.Hero"),
        "the app is NOT recently uninstalled — it is still installed, and \
         offering to reinstall it would be nonsense"
    );

    store.refresh_projection();
    let after = nodes(&store);
    // A Library row carries the app's Details action; the Activity panel's
    // entry for the finished removal is also named "HeroApp" but has none.
    let details: Vec<_> = after
        .iter()
        .filter(|n| {
            n.semantics.role == Role::Button
                && n.semantics.label.as_deref() == Some("Details for HeroApp")
        })
        .map(|n| n.bounds.center())
        .collect();
    let library_rows = after
        .iter()
        .filter(|n| {
            n.semantics.role == Role::ListItem
                && n.semantics.label.as_deref() == Some("HeroApp")
                && details.iter().any(|point| n.bounds.contains(*point))
        })
        .count();
    assert_eq!(
        library_rows,
        1,
        "still exactly one row — now the system one; labels: {:?}",
        labels(&after)
    );
    assert_eq!(
        role_count(&after, Role::Button, "Remove HeroApp"),
        1,
        "the remaining system copy is removable in its own installation; \
         labels: {:?}",
        labels(&after)
    );
    assert_eq!(
        label_count(&after, SYSTEM_MANAGED.label),
        0,
        "a default system installation is not refused; labels: {:?}",
        labels(&after)
    );
    assert_eq!(
        label_count(&after, crate::services::SYSTEM_COPY_REMAINS.label),
        0,
        "and the survival notice is gone: removing the last copy leaves \
         nothing behind; labels: {:?}",
        labels(&after)
    );
    assert!(!store.system_copy_survives_removal("org.test.Hero"));
    assert_eq!(
        store.action_authorization("org.test.Hero", OpKind::Uninstall),
        Authorization::Allowed(MutationScope::SYSTEM)
    );
}

// Repository conflicts require an explicit rename, replace or cancel decision.
