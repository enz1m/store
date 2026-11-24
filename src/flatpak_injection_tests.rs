//! Regression coverage ensuring external Flatpak IDs never become options.

use crate::services::{self, FlatpakExecutor, PermissionOverrideRunner, PermissionToggle};
use crate::{Message, Route, Store};
use iced::advanced::renderer::Headless;
use iced::futures::StreamExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Scratch directory whose lifetime encloses the `Store` that may persist into
/// it. Cleanup runs during normal return and panic unwinding.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "store-flatpak-injection-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create isolated store data directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(Default)]
struct ParsingExecutor {
    calls: Mutex<Vec<Vec<String>>>,
    global_reset: Mutex<bool>,
}

impl FlatpakExecutor for ParsingExecutor {
    fn execute(&self, args: &[String]) -> Result<String, String> {
        self.calls.lock().unwrap().push(args.to_vec());

        let mut after_boundary = false;
        let mut reset = false;
        let mut operands = 0;
        for argument in args.iter().skip(1) {
            if !after_boundary && argument == "--" {
                after_boundary = true;
            } else if !after_boundary && argument == "--reset" {
                reset = true;
            } else if after_boundary || !argument.starts_with('-') {
                operands += 1;
            }
        }
        if args.first().is_some_and(|arg| arg == "override") && reset && operands == 0 {
            *self.global_reset.lock().unwrap() = true;
            return Err("fake Flatpak observed a global reset".into());
        }
        if operands > 0 && !args.iter().any(|argument| argument == "--") {
            return Err("fake Flatpak interpreted an unbounded operand".into());
        }
        Ok("[Context]\nshared=network;\n".into())
    }
}

struct ExecutorRunner(Arc<ParsingExecutor>);

impl PermissionOverrideRunner for ExecutorRunner {
    fn apply(&self, app_id: &str, key: &str, enable: bool) -> Result<(), String> {
        services::apply_permission_override_with(self.0.as_ref(), app_id, key, enable)
    }

    fn reset(&self, app_id: &str) -> Result<(), String> {
        services::reset_permission_overrides_with(self.0.as_ref(), app_id)
    }
}

fn permission_store(app_id: &str, executor: Arc<ParsingExecutor>, data_dir: &TempDir) -> Store {
    let (mut store, _) = Store::boot_settled_at(data_dir.path().to_path_buf(), false, None);
    store.loading_installed = false;
    store.loading_catalog = false;
    store.route = Route::Discover;
    // Enter through the shipped detail-page reducer route. Its background
    // reads are deliberately not executed here; the reset task below is the
    // injected process boundary under test.
    let _detail_tasks = store.update(Message::OpenAppDetail(app_id.into()));
    let _permission_read = store.update(Message::OpenSandboxDialog);
    store.permission_toggles = crate::PermissionTogglesState::Ready(vec![PermissionToggle {
        key: "network".into(),
        label: "Network Access".into(),
        category: "network".into(),
        app_default: true,
        enabled: false,
    }]);
    store.permission_override_runner = Arc::new(ExecutorRunner(executor));
    drop(_permission_read);
    drop(_detail_tasks);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while store.read_owner.dispatcher().snapshot().live() != 0
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        store.read_owner.dispatcher().snapshot().live(),
        0,
        "cancelled fixture reads must settle before the mutation admission positive control"
    );
    store
}

fn task_messages(task: iced::Task<Message>) -> Vec<Message> {
    iced::futures::executor::block_on(async move {
        let Some(mut stream) = iced_test::runtime::task::into_stream(task) else {
            return Vec::new();
        };
        let mut messages = Vec::new();
        while let Some(action) = stream.next().await {
            if let iced_test::runtime::Action::Output(message) = action {
                messages.push(message);
            }
        }
        messages
    })
}

fn rendered_labels(store: &Store) -> Vec<String> {
    let mut renderer = iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer");
    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        store.view(),
        iced::Size::new(900.0, 700.0),
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    let mut collector = enzui_core::collect::SemanticsCollector::new();
    ui.operate(&renderer, {
        let operation: &mut dyn iced::advanced::widget::Operation = &mut collector;
        operation
    });
    collector
        .into_collected()
        .into_iter()
        .filter_map(|node| node.semantics.label)
        .collect()
}

#[test]
fn every_flatpak_effect_boundary_rejects_invalid_app_id_before_execution() {
    let executor = ParsingExecutor::default();
    assert!(services::read_permissions_with_executor(&executor, "--reset", None).is_err());
    assert!(services::read_overrides_with_executor(&executor, "--reset").is_err());
    assert!(
        services::apply_permission_override_with(&executor, "--reset", "network", true).is_err()
    );
    assert!(services::reset_permission_overrides_with(&executor, "--reset").is_err());
    assert!(
        services::add_remote_with(
            &executor,
            "--system",
            "https://example.org/repo",
            services::MutationScope::USER
        )
        .is_err()
    );
    assert!(services::remove_remote_with(&executor, "--system").is_err());
    assert!(executor.calls.lock().unwrap().is_empty());

    let cancellation = services::OperationCancellation::default();
    let mut sink = |_| {};
    let never_spawn = std::path::Path::new("/definitely/not/a/flatpak-executor");
    let invalid = Err("invalid Flatpak application ID".to_owned());
    // Apply the operand screen to both installation scopes; only user scope is mutable here.
    let scope = services::MutationScope::USER;
    assert_eq!(
        services::launch_installed_with(
            never_spawn,
            "--reset",
            &services::InstallScope::User,
            &cancellation
        )
        .map(|_| ()),
        invalid
    );

    assert_eq!(
        services::install_streaming(never_spawn, "--reset", scope, &cancellation, &mut sink),
        invalid
    );
    assert_eq!(
        services::update_streaming(
            never_spawn,
            Some("--reset"),
            scope,
            &cancellation,
            &mut sink
        ),
        invalid
    );
    assert_eq!(
        services::uninstall_streaming(never_spawn, "--reset", scope, &cancellation, &mut sink),
        invalid
    );
}

#[test]
fn option_shaped_literal_is_not_interpreted_as_a_global_reset() {
    let executor = ParsingExecutor::default();
    let mut args = crate::flatpak_args::FlatpakArgs::new("override");
    args.options(["--user", "--reset"]);
    // Bypass ID validation deliberately so this test isolates the operand
    // terminator. The fake parser must treat the second `--reset` as data.
    args.literal_operand("--reset");
    let result = executor.execute(&args.finish());
    assert!(
        !*executor.global_reset.lock().unwrap(),
        "fake Flatpak interpreted the injected ID as a global reset"
    );
    assert!(result.is_ok(), "bounded operand was rejected: {result:?}");
}

#[test]
fn detail_permission_reset_refuses_injected_id_without_global_reset_and_renders_error() {
    let injected = services::FlathubCollectionHit {
        app_id: "--reset".into(),
        name: "Injected".into(),
        ..Default::default()
    };
    assert!(
        services::hit_to_entry(&injected).is_none(),
        "the malicious catalogue hit must be refused during ingestion"
    );
    let data_dir = TempDir::new("invalid-reset");
    let executor = Arc::new(ParsingExecutor::default());
    let mut store = permission_store("--reset", Arc::clone(&executor), &data_dir);

    let task = store.update(Message::ResetPermissionsToDefaults);
    let messages = task_messages(task);
    assert_eq!(messages.len(), 1, "reset task must report one completion");
    for message in messages {
        let _ = store.update(message);
    }

    assert!(
        executor.calls.lock().unwrap().is_empty(),
        "invalid ID reached executor"
    );
    assert!(!*executor.global_reset.lock().unwrap());
    let error = store
        .permission_error()
        .expect("refusal must be visible state");
    assert!(
        error.contains("invalid Flatpak application ID"),
        "{error:?}"
    );
    assert!(
        rendered_labels(&store)
            .iter()
            .any(|label| label.contains("invalid Flatpak application ID")),
        "permission dialog did not render the refusal"
    );
}

#[test]
fn cold_detail_waits_for_catalogue_authority_and_retires_early_results() {
    let data = TempDir::new("cold-detail");
    let (mut store, _) = Store::boot_settled_at(data.path().to_path_buf(), false, None);
    let id = "org.mozilla.firefox";
    store.catalog_index = crate::catalog::CatalogIndex::default();
    store.catalog = Default::default();
    let _initial = store.update(Message::OpenAppDetail(id.into()));
    assert!(!store.detail_metadata_authorized);
    let early = store.detail_generation;
    let cache = store.cache_cap();
    let load = crate::startup_load::load_catalog_with_fixture(cache.as_ref(), true);
    let _loaded = store.apply_catalog_load(load);
    assert!(store.detail_metadata_authorized);
    assert!(store.detail_generation > early);
    assert_eq!(store.detail_app_id.as_deref(), Some(id));
    let current = store.detail_generation;
    let _duplicate = store.resume_catalog_detail();
    assert_eq!(store.detail_generation, current);
    store.route = Route::Discover;
    store.detail_metadata_authorized = false;
    let _departed = store.resume_catalog_detail();
    assert_eq!(store.route, Route::Discover);
    assert_eq!(store.detail_generation, current);
}
