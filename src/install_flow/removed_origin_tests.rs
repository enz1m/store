//! Reinstall requires the same observed repository name, installation and canonical URL.

use super::*;

const LOCAL_ID: &str = "org.enzui.PermissionsQA";
const LOCAL_NAME: &str = "PermissionsQA";
const LOCAL_REMOTE: &str = "enzui-permissions-qa";
const LOCAL_URL: &str = "file:///home/user/qa/permissions-repo";

fn local_remote(url: &str, options: &str) -> RemoteInfo {
    RemoteInfo {
        name: LOCAL_REMOTE.into(),
        title: LOCAL_REMOTE.into(),
        url: url.into(),
        options: options.into(),
    }
}

fn repositories() -> Vec<RemoteInfo> {
    vec![
        remote("flathub", "user"),
        local_remote(LOCAL_URL, "user,no-gpg-verify"),
    ]
}

// Install the local app, remove it through the real confirmation and
// completion, and land on the Library with it under "Recently removed".
fn removed_local_app() -> Store {
    let mut installed = installed_in(LOCAL_ID, LOCAL_NAME, InstallScope::User, LOCAL_REMOTE);
    installed.branch = "master".into();
    installed.deployed_arch = "x86_64".into();
    let mut store = library_store(vec![installed], repositories());
    assert!(
        store.catalog_entry(LOCAL_ID).is_none(),
        "the app must be absent from the catalogue"
    );
    let _ = store.update(Message::UninstallConfirmRequested(
        LOCAL_ID.into(),
        LOCAL_NAME.into(),
    ));
    confirm_pending_uninstall(&mut store);
    let _ = store.update(Message::OperationEvent(crate::flatpak::OpEvent::Finished {
        id: current_operation_id(&store, LOCAL_ID),
        terminal: crate::flatpak::OperationTerminal::Succeeded,
    }));
    assert!(store.installed.is_empty());
    assert_eq!(store.recently_uninstalled.len(), 1);
    store.operations.clear();
    store.refresh_projection();
    // Removal history sits behind the Library's Activity button.
    let _ = store.update(Message::LibraryActivityToggled);
    assert!(store.library_activity_open);
    store
}

fn reinstall_label() -> String {
    format!("Reinstall {LOCAL_NAME}")
}

#[test]
fn recently_removed_local_app_reinstalls_from_its_remote() {
    let mut store = removed_local_app();
    assert_eq!(
        store.install_authorization(LOCAL_ID),
        Authorization::Allowed(MutationScope::USER)
    );
    let page = nodes(&store);
    assert_eq!(
        role_count(&page, Role::Button, &reinstall_label()),
        1,
        "{:?}",
        labels(&page)
    );
    assert_eq!(label_count(&page, REPOSITORY_UNKNOWN.label), 0);

    let _ = store.update(Message::InstallRequested(
        LOCAL_ID.into(),
        LOCAL_NAME.into(),
    ));
    let consent = store
        .install_consent
        .as_ref()
        .expect("consent opens for the recorded remote");
    assert_eq!(consent.remote, LOCAL_REMOTE);
    assert_eq!(consent.source_url.as_deref(), Some(LOCAL_URL));
    crate::install_consent::approve_fixture(&mut store);
    assert!(store.install_consent.is_none());
    assert_eq!(store.operations.len(), 1);
    assert_eq!(store.operations[0].app_id, LOCAL_ID);
    assert_eq!(store.operations[0].kind, OpKind::Install);
    assert!(store.operations[0].error.is_none());
}

#[test]
fn review_resolving_another_address_is_not_confirmable() {
    let mut store = removed_local_app();
    let _ = store.update(Message::InstallRequested(
        LOCAL_ID.into(),
        LOCAL_NAME.into(),
    ));
    let generation = store.install_consent.as_ref().expect("consent").generation;
    let plan = serde_json::json!({"schema_version":1,"review_token":"a".repeat(64),
        "remote":{"name":LOCAL_REMOTE,"url":"file:///elsewhere/repo"},
        "operations":[{"type":"install","ref":format!("app/{LOCAL_ID}/x86_64/master"),"commit":"b".repeat(64),"metadata":{}}],
        "override_sources":[]});
    let _ = store.update(Message::InstallReviewLoaded(generation, Ok(plan)));
    let _ = store.update(Message::InstallConsentConfirm);
    assert!(store.operations.is_empty());
    assert!(
        store
            .install_consent
            .as_ref()
            .expect("consent stays open")
            .error
            .is_some()
    );
    assert!(store.confirmed_install.is_none());
}

#[test]
fn reinstall_review_is_pinned_to_the_removed_branch() {
    let mut store = removed_local_app();
    let _ = store.update(Message::InstallRequested(
        LOCAL_ID.into(),
        LOCAL_NAME.into(),
    ));
    let consent = store.install_consent.as_ref().expect("consent");
    assert_eq!(
        consent.target,
        crate::services::install_review::RefTarget::recorded("master", "x86_64")
    );
    let generation = consent.generation;
    let plan = serde_json::json!({"schema_version":1,"review_token":"a".repeat(64),
        "remote":{"name":LOCAL_REMOTE,"url":LOCAL_URL},
        "operations":[{"type":"install","ref":format!("app/{LOCAL_ID}/x86_64/stable"),"commit":"b".repeat(64),"metadata":{}}],
        "override_sources":[]});
    let _ = store.update(Message::InstallReviewLoaded(generation, Ok(plan)));
    let _ = store.update(Message::InstallConsentConfirm);
    assert!(store.operations.is_empty());
    assert!(
        store
            .install_consent
            .as_ref()
            .expect("consent stays open")
            .error
            .is_some()
    );
}

// Every way the recorded source can stop being the same source keeps the
// pre-existing refusal, in the view, the reducer and consent alike.
fn assert_refused_as_unknown(remotes: Vec<RemoteInfo>, case: &str) {
    let mut store = removed_local_app();
    with_repositories(&mut store, RemoteLoad::Loaded, remotes);
    assert_eq!(
        store.install_authorization(LOCAL_ID),
        Authorization::Refused(REPOSITORY_UNKNOWN),
        "{case}"
    );
    let page = nodes(&store);
    assert_eq!(
        role_count(&page, Role::Button, &reinstall_label()),
        0,
        "{case}"
    );
    assert_eq!(label_count(&page, REPOSITORY_UNKNOWN.label), 1, "{case}");

    let _ = store.update(Message::InstallRequested(
        LOCAL_ID.into(),
        LOCAL_NAME.into(),
    ));
    assert!(
        store.install_consent.is_none(),
        "{case}: no review may open"
    );
    assert_eq!(store.operations.len(), 1, "{case}");
    assert_eq!(
        store.operations[0].error.as_deref(),
        Some(REPOSITORY_UNKNOWN.reason),
        "{case}"
    );

    // Consent resolves the remote on its own; it must not find one either.
    store.operations.clear();
    let _ = store.start_install_consent(LOCAL_ID.into(), LOCAL_NAME.into());
    assert!(
        store.install_consent.is_none(),
        "{case}: consent found a remote"
    );
}

#[test]
fn changed_remote_url_refuses_reinstall() {
    assert_refused_as_unknown(
        vec![
            remote("flathub", "user"),
            local_remote("file:///srv/other-repo", "user,no-gpg-verify"),
        ],
        "URL changed",
    );
}
