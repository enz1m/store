//! Install scope and refusal: unmutable rows, updates by scope, and authorization.

use super::*;

// **The hermeticity gate.** A store built in a test spawns a program that
// cannot exist, so no reducer test can reach the host's Flatpak installation
// even by driving a real Install or Remove message to completion.
//
// The previous revision of this suite let three tests run
// `flatpak <verb> --user -y -- org.test.*` against the developer's machine.
// Those ids do not resolve, so nothing was modified — but "it fails before it
// can do damage" is a different promise from "it cannot run", and a suite
// whose behaviour depends on the host not having an app called `org.test.Hero`
// answers differently on someone else's machine.
//
#[test]
fn no_store_test_can_reach_the_host_flatpak() {
    let store = browse_store();
    assert_eq!(
        store.flatpak_program,
        std::path::Path::new(crate::services::NO_FLATPAK_PROGRAM),
        "a test store must spawn the absent guard program, never the host's \
         flatpak"
    );
    assert!(
        !store.flatpak_program.exists(),
        "the guard program must not exist on this machine: {}",
        store.flatpak_program.display()
    );
}

#[test]
fn a_system_only_repository_offers_no_install_and_the_reducer_refuses_it() {
    let mut store = browse_store();
    // The catalogue's origin exists only in the system scope.
    with_repositories(
        &mut store,
        RemoteLoad::Loaded,
        vec![remote("flathub", "system")],
    );

    assert_eq!(
        store.install_authorization("org.test.Hero"),
        Authorization::Refused(SYSTEM_REMOTE_ONLY)
    );

    // Discover, Search and the detail page must all withhold the control.
    let _ = store.update(Message::Navigate(Route::Discover));
    let discover = nodes(&store);
    assert_eq!(
        role_count(&discover, Role::Button, "Install HeroApp"),
        0,
        "Discover must not offer an install that cannot resolve a ref; \
         labels: {:?}",
        labels(&discover)
    );
    assert!(
        label_count(&discover, SYSTEM_REMOTE_ONLY.label) >= 1,
        "and it must say why; labels: {:?}",
        labels(&discover)
    );

    let _ = store.update(Message::OpenAppDetail("org.test.Hero".into()));
    let detail = nodes(&store);
    assert_eq!(
        role_count(&detail, Role::Button, "Install HeroApp"),
        0,
        "labels: {:?}",
        labels(&detail)
    );
    assert_eq!(label_count(&detail, SYSTEM_REMOTE_ONLY.label), 1);

    // Second gate: the message arrives anyway (an accessibility action, or a
    // frame built before the repository list landed). It is refused, with the
    // reason, and no operation is left running to spawn a child.
    let _ = store.update(Message::InstallRequested(
        "org.test.Hero".into(),
        "HeroApp".into(),
    ));
    crate::install_consent::approve_fixture(&mut store);
    let op = store
        .operation_for("org.test.Hero")
        .expect("the refusal is reported, not dropped");
    assert!(op.finished, "a refused operation is terminal immediately");
    assert_eq!(op.message, "Refused");
    assert_eq!(op.error.as_deref(), Some(SYSTEM_REMOTE_ONLY.reason));
    assert!(
        !store.is_installed("org.test.Hero"),
        "nothing was installed"
    );
}

// The **positive** install control: a user-scope repository keeps Install
// offered and resolves it to exactly `--user`.
//
// Asserted on the resolution rather than by spawning, so the exact flag is
// pinned with no child at all. `flatpak::every_operation_takes_its_scope_option…`
// closes the remaining link from `Operation.scope` to the recorded argv.
//
#[test]
fn a_user_repository_keeps_install_offered_and_resolves_to_exactly_user_scope() {
    let mut store = browse_store();
    // The same remote in BOTH scopes, which is the ordinary layout: the user
    // one wins because it is the one this build can install from.
    with_repositories(
        &mut store,
        RemoteLoad::Loaded,
        vec![remote("flathub", "system"), remote("flathub", "user")],
    );

    assert_eq!(
        store.install_authorization("org.test.Hero"),
        Authorization::Allowed(MutationScope::USER),
        "user scope is preferred when a repository is configured in both"
    );

    let _ = store.update(Message::OpenAppDetail("org.test.Hero".into()));
    let ns = nodes(&store);
    assert_eq!(
        role_count(&ns, Role::Button, "Install HeroApp"),
        1,
        "labels: {:?}",
        labels(&ns)
    );
    assert_eq!(label_count(&ns, SYSTEM_REMOTE_ONLY.label), 0);

    // The exact argv the resolved scope produces, with no child process.
    let scope = store
        .install_authorization("org.test.Hero")
        .scope()
        .expect("allowed");
    assert_eq!(scope.option(), "--user");
    assert_eq!(
        crate::services::mutation_args("install", Some("org.test.Hero"), scope).expect("argv"),
        ["install", "--user", "-y", "--", "org.test.Hero"]
            .map(str::to_owned)
            .to_vec()
    );
}

// "Update all" starts only the updates this build can authorize, and leaves
// the rest visible rather than queueing refusals nobody asked for.
//
#[test]
fn update_all_starts_only_the_scopes_this_build_can_authorize() {
    let mut store = browse_store();
    let _ = store.update(Message::InstalledLoaded(Ok(vec![
        installed_in("org.test.UserApp", "UserApp", InstallScope::User, "flathub"),
        installed_in(
            "org.test.SystemApp",
            "SystemApp",
            InstallScope::System,
            "flathub",
        ),
        installed_in(
            "org.test.UnknownApp",
            "UnknownApp",
            InstallScope::Unknown,
            "flathub",
        ),
        installed_in(
            "org.test.NamedApp",
            "NamedApp",
            InstallScope::Named("extra".into()),
            "flathub",
        ),
    ])));
    store.deliver_known_updates(vec![
        AvailableUpdate::user("org.test.UserApp"),
        AvailableUpdate::system("org.test.SystemApp"),
        AvailableUpdate::system("org.test.UnknownApp"),
        AvailableUpdate::in_scope("org.test.NamedApp", InstallScope::Named("extra".into())),
    ]);
    assert_eq!(
        store.updates_available().len(),
        4,
        "every scope's pending update stays visible"
    );

    let _ = store.update(Message::UpdateAllRequested);
    crate::install_consent::approve_fixture(&mut store);
    assert_eq!(
        store
            .operations
            .iter()
            .map(|op| op.app_id.as_str())
            .collect::<Vec<_>>(),
        ["org.test.UserApp"],
        "Update all must not queue work that could only be refused"
    );
    assert_eq!(
        store.operations[0].error, None,
        "the one started update is a real operation, not a refusal"
    );
    // Updates run one at a time; the system-only app's update is queued next,
    // and the unknown- and named-installation updates are not queued at all.
    assert_eq!(
        store
            .pending_update_reviews
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["org.test.SystemApp"]
    );
}
