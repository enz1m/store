//! Install lifecycle and pushed-page navigation, gated through the real
//! reducer and `Store::view`: an accepted install keeps its route and replaces
//! its own action with a determinate control, catalogue rows agree with the
//! installed list, and every pushed page shares one titlebar Back.

use crate::model::{CancelState, OpKind, OperationId, OperationState};
use crate::services::AppEntry;
// ---------------------------------------------------------------------------

use crate::services::{
    Authorization, AvailableUpdate, InstallScope, MutationScope, REPOSITORY_UNKNOWN, RemoteInfo,
    RemoteLoad, SYSTEM_MANAGED, SYSTEM_REMOTE_ONLY,
};
use crate::{Message, Route, Store};

use enzui_core::collect::{CollectedNode, SemanticsCollector};
use enzui_core::semantics::Role;

const DESKTOP: iced::Size = iced::Size {
    width: 1400.0,
    height: 900.0,
};

fn nodes(store: &Store) -> Vec<CollectedNode> {
    nodes_at(store, DESKTOP)
}

fn current_operation_id(store: &Store, app_id: &str) -> OperationId {
    store.operation_for(app_id).expect("operation").id()
}

fn nodes_at(store: &Store, size: iced::Size) -> Vec<CollectedNode> {
    use iced::advanced::renderer::Headless;
    use iced::advanced::widget::Operation;

    let mut renderer = iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer");

    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        Store::view(store),
        size,
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    let mut collector = SemanticsCollector::new();
    ui.operate(&renderer, {
        let op: &mut dyn Operation = &mut collector;
        op
    });
    crate::i18n::test_nodes(collector.into_collected())
}

fn labels(nodes: &[CollectedNode]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| node.semantics.label.clone())
        .collect()
}

fn role_count(nodes: &[CollectedNode], role: Role, label: &str) -> usize {
    nodes
        .iter()
        .filter(|n| n.semantics.role == role && n.semantics.label.as_deref() == Some(label))
        .count()
}

fn app(id: &str, name: &str) -> AppEntry {
    AppEntry {
        id: id.into(),
        name: name.into(),
        summary: "A fixture app".into(),
        downloads: 900_000,
        category: "game".into(),
        // Catalogue fixtures need an origin so authorization never guesses a repository or scope.
        origin: crate::services::FLATHUB_REPOSITORY.origin.into(),
        ..Default::default()
    }
}

fn browse_store() -> Store {
    let (mut s, ()) = Store::boot_for_test();
    s.loading_installed = false;
    s.loading_catalog = false;
    s.catalog.popular = vec![
        app("org.test.Hero", "HeroApp"),
        app("org.test.Second", "SecondApp"),
        app("org.test.Third", "ThirdApp"),
    ];
    s.catalog.trending = vec![app("org.test.Feed", "FeedApp")];
    s.route = Route::Discover;
    s.refresh_projection();
    s
}

// [`app`] installed in a named Flatpak installation, from a named repository.
fn installed_in(id: &str, name: &str, scope: InstallScope, origin: &str) -> AppEntry {
    AppEntry {
        installed: true,
        install_scope: scope,
        origin: origin.into(),
        ..app(id, name)
    }
}

fn installed_user(id: &str, name: &str) -> AppEntry {
    installed_in(
        id,
        name,
        InstallScope::User,
        crate::services::FLATHUB_REPOSITORY.origin,
    )
}

// A configured repository in a named scope, publishing from Flathub's URL so
// the rename-matching arm is reachable by name *or* URL.
fn remote(name: &str, options: &str) -> RemoteInfo {
    RemoteInfo {
        name: name.into(),
        title: name.into(),
        url: crate::services::FLATHUB_REPOSITORY.url.into(),
        options: options.into(),
    }
}

// Declare the repository read's answer on a test store.
//
// The parallel of `loading_installed = false` / `loading_catalog = false` for
// the third read. `browse_store` starts from the `cfg(test)` default —
// loaded, with the declared fixture repositories — and every test that means
// a different answer says so through this.
fn with_repositories(
    store: &mut Store,
    load: crate::services::RemoteLoad,
    remotes: Vec<RemoteInfo>,
) {
    store.remotes = remotes;
    store.remotes_load = load;
    store.refresh_projection();
}

// A Library route showing exactly `rows`, reached through the real navigation
// arm so a measurement cannot pass with the shell's Library arm severed.
fn library_store(rows: Vec<AppEntry>, remotes: Vec<RemoteInfo>) -> Store {
    let mut store = browse_store();
    with_repositories(&mut store, crate::services::RemoteLoad::Loaded, remotes);
    let _ = store.update(Message::InstalledLoaded(Ok(rows)));
    let _ = store.update(Message::Navigate(Route::Library));
    store.refresh_projection();
    store
}

// How many nodes carry `label`, whatever their role. A refusal disclosure is
// a `Role::Label` chip, not a control.
fn label_count(nodes: &[CollectedNode], label: &str) -> usize {
    nodes
        .iter()
        .filter(|node| node.semantics.label.as_deref() == Some(label))
        .count()
}

// Confirm a pending uninstall through the real dialog resolution.
fn confirm_pending_uninstall(store: &mut Store) {
    let _ = store.update(Message::UninstallConfirmEvent(
        enzui_widgets::confirm_dialog::ConfirmDialogEvent::Resolved(
            enzui_core::modal::DismissCause::DefaultAction,
        ),
    ));
}

// The same app installed in **both** installations, so `installed_scope`
// resolves to `User` — the precondition for the defect below.
fn installed_in_both(id: &str, name: &str) -> Vec<AppEntry> {
    vec![
        installed_in(id, name, InstallScope::System, "flathub"),
        installed_in(id, name, InstallScope::User, "flathub"),
    ]
}

fn live_verified() -> crate::services::PublisherVerification {
    crate::services::PublisherVerification::Verified {
        method: "website".into(),
        provenance: crate::services::VerificationProvenance::Live,
    }
}

#[path = "install_flow/dual_install_tests.rs"]
mod dual_install_tests;
#[path = "install_flow/lifecycle_tests.rs"]
mod lifecycle_tests;

#[path = "install_flow/removed_origin_tests.rs"]
mod removed_origin_tests;
#[path = "install_flow/scope_tests.rs"]
mod scope_tests;
#[path = "install_flow/verification_tests.rs"]
mod verification_tests;
