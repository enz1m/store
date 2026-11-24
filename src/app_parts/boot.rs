//! Store boot constructors.

use super::*;

use iced::Task;

impl Store {
    pub(crate) fn boot() -> (Self, Task<Message>) {
        // Env is read exactly once, on this thread, and the resolved values are
        // handed onward as data. Both durable tiers are resolved here and owned
        // by the returned store; an unavailable tier disables only the
        // operations that require it.
        let fixture_settled = fixture_settled_env(std::env::var("ENZUI_FIXTURE").ok());
        let hero_env = std::env::var("ENZUI_FIXTURE_HERO").ok();
        let capabilities = crate::roots::StoreRootResolver::from_env().resolve();
        #[cfg_attr(
            test,
            allow(
                unused_mut,
                reason = "production attaches desktop services after fixture construction"
            )
        )]
        let (mut store, task) = Self::boot_core(capabilities, fixture_settled, hero_env);

        #[cfg(not(test))]
        {
            store.shell_activity = crate::shell_activity::ShellActivity::production();
        }
        #[cfg(all(feature = "center", not(test)))]
        store.start_center();
        (store, task)
    }

    // Explicit fixture selection avoids changing global environment in concurrent tests.
    #[cfg(test)]
    pub(crate) fn boot_settled(fixture_settled: bool) -> (Self, Task<Message>) {
        Self::boot_settled_at(services::dirs_fallback(), fixture_settled, None)
    }

    // A test boot at a literal scratch data directory: the injected roots are
    // installed for the stateless service helpers, then the shared core runs.
    #[cfg(test)]
    pub(crate) fn boot_settled_at(
        data_dir: std::path::PathBuf,
        fixture_settled: bool,
        hero_env: Option<String>,
    ) -> (Self, Task<Message>) {
        Self::boot_core(
            crate::roots::StoreCapabilities::for_test(&data_dir),
            fixture_settled,
            hero_env,
        )
    }

    // The shared boot body, driven by which durable tiers resolved.
    //
    // The store owns the resolved capabilities. When the data tier is
    // unavailable the recent-search file binds no authority; when the cache
    // tier is unavailable the catalog, icon, screenshot, and featured caches
    // are skipped. Flatpak and network reads run regardless of the cache tier.
    pub(crate) fn boot_core(
        capabilities: roots::StoreCapabilities,
        fixture_settled: bool,
        hero_env: Option<String>,
    ) -> (Self, Task<Message>) {
        #[cfg(not(test))]
        let (themes, theme_boot) = ThemeIntegration::boot();
        #[cfg(test)]
        let (themes, theme_boot) = test_theme_integration(EnzTheme::dark());

        // Optional CLI arguments, for visual verification only. They select a
        // starting view and pre-fill the search box; they cannot start an
        // operation, and the destructive-op guard is unaffected.
        let args: Vec<String> = std::env::args().skip(1).collect();
        let (
            boot_route,
            boot_repositories,
            boot_category,
            boot_query,
            boot_detail,
            boot_add_remote,
        ) = parse_args(&args);

        // The recent-search history is state under the validated data root; a
        // store with no data tier binds no authority and reads nothing.
        let (recent_searches, recent_searches_unavailable, recent_searches_file) =
            match capabilities.data() {
                Some(data) => {
                    let path = data.recent_searches_path();
                    let acquire = services::acquire_recent_searches(&path);
                    let (history, unavailable) = match acquire.outcome() {
                        services::RecentSearchesLoad::Loaded(history) => (history.clone(), None),
                        services::RecentSearchesLoad::Unavailable(reason) => {
                            (Vec::new(), Some(*reason))
                        }
                    };
                    (
                        history,
                        unavailable,
                        services::RecentSearchesFile::from_acquire(path, acquire),
                    )
                }
                None => (
                    Vec::new(),
                    None,
                    services::RecentSearchesFile::unavailable(),
                ),
            };

        // One bounded pool owns every off-thread catalogue read for this store.
        // A spawn failure yields a permanently shut-down owner whose
        // submissions settle as unavailable rather than an unbounded thread.
        let catalog_owner = blocking_owner(
            "store-catalog",
            CATALOG_WORKER_COUNT,
            CATALOG_QUEUE_CAPACITY,
        );

        // A spawn failure yields a permanently shut-down pool whose submissions
        // settle as a typed refusal, never an unbounded thread.
        let export_owner =
            StoreExportOwner::start().unwrap_or_else(|_| StoreExportOwner::unavailable());

        // One bounded pool owns every short off-thread flatpak read for this
        // store. A spawn failure yields a permanently shut-down owner whose
        // submissions settle as a typed refusal rather than an unbounded thread.
        let read_owner = blocking_owner("store-read", READ_WORKER_COUNT, READ_QUEUE_CAPACITY);
        let detail_owner =
            blocking_owner("store-detail", DETAIL_WORKER_COUNT, DETAIL_QUEUE_CAPACITY);
        let mutation_owner = blocking_owner(
            "store-mutation",
            OPERATION_CONCURRENCY_LIMIT,
            OPERATION_QUEUE_CAPACITY,
        );

        let mut store = Self {
            instance: enzui_widgets::single_instance::SingleInstance::standalone(),
            route: boot_route,
            // With no route history, Back must remain on the startup route.
            previous_route: boot_route,
            browse_category: boot_category,
            expanded_collection: None,
            featured_page: None,
            featured_images: std::collections::HashMap::new(),
            featured_brands: std::collections::HashMap::new(),
            featured_generations: std::collections::HashMap::new(),
            featured_deferred: Vec::new(),
            featured_generation: 0,
            detail_app_id: boot_detail.clone(),
            detail_origin: "flathub".to_owned(),
            detail_scope: None,
            detail_scope_identity: "unknown".to_owned(),
            detail_metadata_authorized: false,
            detail_cancellation: services::OperationCancellation::default(),
            detail_permissions: PermissionRead::Loading,
            detail_permissions_await_remotes: false,
            permissions_dialog_open: false,
            permissions_focus: None,
            ratings_breakdown_open: false,
            screenshots: Vec::new(),
            screenshot_viewer: None,
            screenshot_load_state: ScreenshotLoadState::Loading,
            detail_generation: u64::from(boot_detail.is_some()),
            detail_generation_exhausted: false,
            detail_request_error: None,
            detail_context: None,
            detail_settled_at: None,
            verification_settled_at: None,
            release_history_open: false,
            release_history_focus: None,
            permission_toggles: PermissionTogglesState::Loading,
            shared_permission_snapshot: None,
            shared_permission_details: false,
            permission_toggles_read_request: 0,
            sandbox_dialog_open: false,
            sandbox_dialog_focus: None,
            permission_error: None,
            permission_error_detail: None,
            installed: Vec::new(),
            installed_known: false,
            pending_update_scan: None,
            installed_read_request: 0,
            update_discovery: services::UpdateDiscovery::Loading,
            updates_generation: 0,
            #[cfg(feature = "center")]
            center: None,
            updates_read_request: 0,
            remotes: default_remotes(),
            enzimos_apps: Vec::new(),
            remotes_read_request: 0,
            remotes_load: default_remotes_load(),
            add_remote_form: if boot_add_remote {
                Some(AddRemoteForm::default())
            } else {
                None
            },
            add_remote_focus: None,
            repositories_focus: None,
            add_remote_generation: 0,
            catalog: services::FlathubCatalogs::default(),
            catalog_index: catalog::CatalogIndex::default(),
            catalog_stale: false,
            catalog_error: None,
            catalog_cache_load_error: None,
            recently_uninstalled: Vec::new(),
            search_query: boot_query,
            titlebar_overflow_open: false,
            library_query: String::new(),
            recent_searches,
            recent_searches_unavailable,
            recent_searches_file,
            recent_searches_generation: 0,
            recent_searches_loading: false,
            search_popup_open: false,
            library_sort_menu_open: false,
            search_popup_results: Vec::new(),
            search_popup_total: 0,
            icons_requested: std::collections::HashSet::new(),
            recent_searches_reload_slot: std::sync::Arc::new(std::sync::Mutex::new(None)),
            recent_search_refusal: None,
            recent_searches_save_error: None,
            filters: SearchFilters::default(),
            library_sort: LibrarySort::default(),
            automatic_updates: capabilities
                .data()
                .is_none_or(roots::DataCapability::automatic_updates),
            automatic_updates_error: None,
            #[cfg(not(test))]
            automatic_updater: std::path::Path::new(AUTOMATIC_UPDATER).exists(),
            #[cfg(test)]
            automatic_updater: true,
            catalog_sort: model::CatalogSort::default(),
            library_activity_open: false,
            library_scroll: Default::default(),
            search_scroll: Default::default(),
            category_scroll: Default::default(),
            projection: std::cell::OnceCell::new(),
            projection_builds: std::cell::Cell::new(0),
            adopted_rows: std::cell::OnceCell::new(),
            library_cache: SubtreeHandle::new(SubtreeId::new("store-library")),
            library_operations_signature: 0,
            library_realised_at: std::rc::Rc::new(std::cell::Cell::new(f32::NAN)),
            operations: Vec::new(),
            operation_cancels: std::collections::HashMap::new(),
            operation_completions: std::collections::HashMap::new(),
            operation_attempts: std::collections::HashMap::new(),
            shell_activity: Default::default(),
            activity_center: {
                let mut center = ActivityCenter::new();
                center.register_provider(STORE_ACTIVITY_PROVIDER, true);
                center
            },
            activity_schedule: ActivitySchedule::default(),
            activity_clock: std::time::Instant::now(),
            next_activity_id: 0,
            active_activity_keys: std::collections::HashMap::new(),
            pending_uninstall: None,
            pending_update_reviews: std::collections::VecDeque::new(),
            install_consent: None,
            install_review_generation: 0,
            confirmed_install: None,
            // Tests drive the reviewed path; they set this off to drive the direct one.
            #[cfg(not(test))]
            permission_service: services::install_review::service_available(),
            #[cfg(test)]
            permission_service: true,
            confirm_state: enzui_core::confirm::ConfirmState::new(None),
            pending_remote_removal: None,
            remote_confirm_state: enzui_core::confirm::ConfirmState::new(None),
            loading_installed: true,
            loading_catalog: true,
            last_error: None,
            launching_app: None,
            last_url_error: None,
            url_launcher: url_launch::UrlLauncher::new(),
            window: None,
            chrome: enzui_widgets::app_shell::ShellWindowState::deferred(),
            closing: false,
            closing_window: None,
            close_drain_session: None,
            close_retained_owners: None,
            close_drain_probe: None,
            close_drain_report: None,
            close_operation_ids: Vec::new(),
            close_helper_status: CloseHelperStatus::NotStarted,
            close_started: None,
            close_emitted: false,
            close_keep_open_requested: false,
            close_recovery_error: None,
            toasts: enzui_widgets::toast_host::ToastHost::new().id_prefix("store-toast"),
            toast_expiry: enzui_widgets::toast_host::ToastExpirySchedule::default(),
            toasted_failures: Vec::new(),
            close_recovery_toast: None,
            close_failure_dismissed: None,
            catalog_source: std::sync::Arc::new(catalog::HttpCatalogSource::new()),
            permission_override_runner: services::default_permission_override_runner(),
            flatpak_program: services::default_flatpak_program(),
            themes,
            capabilities,
            catalog_owner,
            catalog_generation: 0,
            export_owner,
            parent_export_generation: 0,
            detail_owner,
            read_owner,
            next_owned_read_request: 0,
            deferred_owned_reads: DeferredOwnedReads::default(),
            mutation_owner,
            recent_searches_read_busy: false,
        };

        // Fixture-only settled boot completes reads before painting; normal boot schedules reads without adding repositories.
        let mut tasks: Vec<Task<Message>> = Vec::new();
        // Flatpak and network reads run regardless of the cache tier; the cache
        // read/write inside the catalog load is skipped when the cache tier is
        // unavailable, so a missing cache never disables the catalogue.
        if fixture_settled {
            settle_fixture_now(&mut store, hero_env);
        } else {
            // The capture harness rejects unsettled frames when CAPTURE_REQUIRE=settled.
            eprintln!(
                "[CAPTURE_BUSY] store-app: fixture not settled — \
                 dispatching async catalog/installed reads; first frame will \
                 show the loading placeholder"
            );
            tasks.push(store.start_installed_read());
            tasks.push(store.start_remotes_read());
            tasks.push(store.start_update_check());
            tasks.push(store.dispatch_catalog_cache_load());
        }
        tasks.push(theme_boot.map(|event| Message::Theme(Box::new(event))));
        if boot_repositories {
            tasks.push(store.open_repositories_dialog());
        }
        if store.add_remote_form.is_some() {
            let (lifecycle, open_op) = enzui_widgets::modal_host::ModalFocusLifecycle::open::<
                Message,
            >(ui::remotes::ADD_REMOTE_SCOPE, None);
            store.add_remote_generation = 1;
            store.add_remote_focus = Some(lifecycle);
            tasks.push(iced::advanced::widget::operate(open_op));
        }
        // Booting straight onto the detail page (`detail=<id>`, verification
        // only) uses the same enrichment as `OpenAppDetail`. Under the capture
        // fixture it is folded in synchronously, just like the four boot reads:
        // capture.sh shoots first paint, so dispatching even an immediate
        // fixture through background tasks would reintroduce a loading-frame
        // race. A normal user still gets the asynchronous live reads.
        if let Some(app_id) = boot_detail {
            if fixture_settled {
                settle_detail_fixture_now(&mut store);
            } else {
                let cache_admitted = store
                    .cache_cap()
                    .is_none_or(|cache| cache.admit_detail_generation(store.detail_generation));
                if cache_admitted && store.detail_generation != 0 {
                    tasks.extend(store.detail_open_tasks(
                        store.cache_cap(),
                        app_id,
                        store.detail_origin.clone(),
                        store.detail_scope,
                        store.detail_scope_identity.clone(),
                        store.detail_metadata_authorized,
                        store.detail_generation,
                        store.detail_cancellation.clone(),
                        store.detail_owner.dispatcher(),
                    ));
                } else {
                    store.refuse_detail_generation();
                    store.detail_permissions = PermissionRead::Unavailable;
                    store.detail_context = Some(services::DetailContext::failed());
                    store.screenshot_load_state = ScreenshotLoadState::Failed;
                }
            }
        }
        (store, Task::batch(tasks))
    }

    /// Whether the recent-search history has no bound publication authority, so
    /// automatic saves are blocked and the search UI must offer retry/retire.
    pub(crate) fn recent_searches_blocked(&self) -> bool {
        self.recent_searches_file.is_blocked()
    }

    // A clone of the cache-tier capability, or `None` when the cache tier is
    // unavailable. The clone is cheap and is moved into the exact task closure
    // that performs a cache operation.
    pub(crate) fn cache_cap(&self) -> Option<roots::CacheCapability> {
        self.capabilities.cache().cloned()
    }

    // A clone of the data-tier capability, or `None` when the data tier is
    // unavailable.
    pub(crate) fn data_cap(&self) -> Option<roots::DataCapability> {
        self.capabilities.data().cloned()
    }
}

/// enzimOS's background updater, which honours the automatic-updates setting.
#[cfg(not(test))]
const AUTOMATIC_UPDATER: &str = "/usr/lib/enzimos-store/flatpak-auto-update";
