//! Store application state.

use super::*;

pub(crate) struct Store {
    /// Framework-owned launch forwarding and activation.
    pub(crate) instance: enzui_widgets::single_instance::SingleInstance,
    pub(crate) pending_update_reviews: std::collections::VecDeque<(String, String)>,
    pub(crate) install_consent: Option<crate::install_consent::Consent>,
    pub(crate) install_review_generation: u64,
    pub(crate) confirmed_install: Option<services::install_review::ReviewedInstall>,
    /// Whether the enzimOS permission service is installed; without it
    /// installs and updates go to Flatpak directly, without a review.
    pub(crate) permission_service: bool,
    // nav
    pub(crate) route: Route,
    pub(crate) previous_route: Route,
    pub browse_category: Option<String>,
    /// `None` shows sectioned Discover; a collection opens its full list.
    pub expanded_collection: Option<model::CollectionKind>,
    /// Explicit carousel page; `None` delegates the initial page to framework paging.
    pub featured_page: Option<enzui_core::paging::PageId>,
    // Per-app screenshot lifecycle. Only pages the user has viewed are loaded;
    // catalogue adoption prunes entries outside the current three-page hero.
    pub(crate) featured_images: std::collections::HashMap<String, featured_image::HeroImageState>,
    /// Each featured app's brand colours (light, dark), for its banner.
    pub(crate) featured_brands: std::collections::HashMap<String, ([u8; 3], [u8; 3])>,
    // The live attempt generation for each hero the pool is loading or holding
    // for retry. A settlement is honoured only while it names the app's
    // current generation, and catalogue adoption retires entries outside the
    // current hero pages so a late load cannot revive a dropped page.
    pub(crate) featured_generations: std::collections::HashMap<String, u64>,
    // Hero intents the pool refused as Busy, awaiting one retry tick each,
    // order-preserving and deduplicated.
    pub(crate) featured_deferred: Vec<String>,
    // Source of monotone hero attempt generations.
    pub(crate) featured_generation: u64,
    pub(crate) detail_app_id: Option<String>,
    pub(crate) detail_origin: String,
    pub(crate) detail_scope: Option<services::MutationScope>,
    pub(crate) detail_install_scope: services::InstallScope,
    pub(crate) detail_metadata_authorized: bool,
    pub(crate) detail_cancellation: services::OperationCancellation,

    // Permission reads run asynchronously; the view never launches Flatpak.
    pub(crate) detail_permissions: PermissionRead,
    pub(crate) detail_permissions_await_remotes: bool,
    pub(crate) permissions_dialog_open: bool,
    // Focus owner stored for exactly the live read-only permissions dialog.
    pub(crate) permissions_focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,

    pub(crate) ratings_breakdown_open: bool,

    // Cluster A: the app-detail enrichment (release history, links). Loaded
    // asynchronously when the detail page opens via `fetch_detail_context_cancellable`, so
    // `view` never shells out. `None` until the read lands.
    pub(crate) detail_context: Option<services::DetailContext>,
    pub(crate) detail_settled_at: Option<u64>,
    // Activity-clock timestamp for the current detail verification; absent ages cannot preserve stale results.
    pub(crate) verification_settled_at: Option<u64>,
    // Whether the "Changelogs" release-history dialog is open (Cluster A).
    pub(crate) release_history_open: bool,
    pub(crate) release_history_focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,

    // Screenshots load asynchronously; the optional index identifies the shot
    // enlarged in the modal viewer.
    pub(crate) screenshots: Vec<services::Screenshot>,
    pub(crate) screenshot_viewer: Option<usize>,
    pub(crate) screenshot_load_state: ScreenshotLoadState,
    pub(crate) detail_generation: u64,
    pub(crate) detail_generation_exhausted: bool,
    pub(crate) detail_request_error: Option<&'static str>,

    // Cluster B: the "Sandbox Permissions" editing dialog. `permission_toggles`
    // is the tri-state toggle set (declared default + effective grant) loaded
    // when the dialog opens; `sandbox_dialog_open` gates its presentation.
    pub(crate) permission_toggles: PermissionTogglesState,
    pub(crate) shared_permission_snapshot: Option<Box<serde_json::Value>>,
    pub(crate) shared_permission_details: bool,
    pub(crate) permission_toggles_read_request: u64,
    pub(crate) sandbox_dialog_open: bool,
    pub(crate) sandbox_dialog_focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,
    // Public override errors exclude backend paths, control characters and raw diagnostics.
    pub(crate) permission_error: Option<String>,
    // The escaped diagnostic sink for the last permission failure.
    // Raw backend detail reaches only here — stripped of control/escape
    // sequences and bounded — never the rendered semantic surface above.
    pub(crate) permission_error_detail: Option<String>,

    // data
    pub installed: Vec<AppEntry>,
    /// The installed list has loaded at least once; until then an update scan
    /// cannot be filtered to installed apps without falsely claiming "up to date".
    pub(crate) installed_known: bool,
    /// An update scan that arrived before the installed list, applied when it lands.
    pub(crate) pending_update_scan: Option<(u64, Box<services::UpdateScan>)>,
    pub(crate) installed_read_request: u64,
    /// Updates grouped by app ID, retaining every installation scope and discovery failures.
    pub update_discovery: services::UpdateDiscovery,
    /// Only results matching this generation may replace the current update answer.
    pub updates_generation: u64,
    // Optional process-wide agent session.
    #[cfg(feature = "center")]
    pub(crate) center: Option<enzui_agent::center::CenterHost<crate::center::CenterCall>>,
    pub(crate) updates_read_request: u64,
    pub remotes: Vec<RemoteInfo>,
    pub(crate) enzimos_apps: Vec<AppEntry>,
    pub(crate) remotes_read_request: u64,
    /// Repository read state; installing requires a positively known, usable repository.
    pub remotes_load: services::RemoteLoad,
    /// The add-remote form, present only while its dialog is open (cluster 1).
    /// `None` when closed, so a dismissed dialog holds no draft.
    pub add_remote_form: Option<AddRemoteForm>,
    // Focus owner and generation for the live Add Remote presentation.
    pub(crate) add_remote_focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,
    /// The Repositories dialog's focus owner; the dialog is open while set.
    pub(crate) repositories_focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,
    pub(crate) add_remote_generation: u64,
    /// Discover ranking rails derived from the searchable catalogue.
    pub catalog: services::FlathubCatalogs,
    /// Searchable catalogue; Discover uses its separately ranked rails.
    pub catalog_index: catalog::CatalogIndex,
    /// True when [`Store::catalog_index`] is a cache shown because the refresh
    /// failed — the UI must say so rather than pass it off as current.
    pub catalog_stale: bool,
    /// Why the last catalogue refresh failed. Rendered as an actionable error
    /// only when the index is also empty (nothing to fall back on).
    pub catalog_error: Option<String>,
    /// Safe feedback when the persisted catalogue cache was refused or
    /// malformed. This remains independent of a simultaneous ingest failure.
    pub catalog_cache_load_error: Option<String>,
    pub recently_uninstalled: Vec<AppEntry>,

    // input
    pub search_query: String,
    /// Whether the framework titlebar command menu is open.
    pub titlebar_overflow_open: bool,
    /// The Library-pane filter query. Filters the *installed* list by name/id
    /// only — it is a local filter over apps already on disk, distinct from the
    /// top-bar Flathub search (`search_query`). Its field carries the
    /// "Search installed apps" placeholder (Cluster A).
    pub library_query: String,
    /// Persisted recent-search history, newest first (Cluster A). Loaded at
    /// boot from `~/.local/share/store-app/recent-searches.txt` and rewritten
    /// whenever a query is submitted, so it survives a restart.
    pub recent_searches: Vec<String>,
    /// Whether the titlebar's search popup is open.
    pub(crate) search_popup_open: bool,
    /// Whether the Library's sort menu is open.
    pub(crate) library_sort_menu_open: bool,
    /// The popup's best matches for `search_query`, and how many there are.
    pub(crate) search_popup_results: Vec<services::AppEntry>,
    pub(crate) search_popup_total: usize,
    /// Apps whose icons a list has asked for, so scrolling asks only once.
    pub(crate) icons_requested: std::collections::HashSet<String>,
    /// Set when the recent-search file was refused whole; automatic writes are
    /// blocked and the search UI offers retry/retire (Cluster A recovery).
    pub recent_searches_unavailable: Option<services::RecentSearchesUnavailable>,
    // The identity-bound publication authority bound to the boot read, never
    // re-derived from the pathname on an automatic save.
    pub(crate) recent_searches_file: services::RecentSearchesFile,
    // Reload epoch: a reread carries the epoch it was dispatched under and is
    // discarded on completion if the epoch has since advanced.
    pub(crate) recent_searches_generation: u64,
    /// Whether an off-thread reread worker is active; a second retry is refused
    /// while set so retries cannot multiply workers.
    pub recent_searches_loading: bool,
    // Handoff from the off-thread reread worker: the bound authority never
    // crosses a `Message`.
    pub(crate) recent_searches_reload_slot:
        std::sync::Arc<std::sync::Mutex<Option<services::RecentSearchesAcquire>>>,
    /// Transient refusal for an over-limit submitted query, cleared on the next
    /// accepted submission (Cluster A).
    pub recent_search_refusal: Option<services::RecentSearchRefusal>,
    /// Visible message for a publication failure on save or retire.
    pub recent_searches_save_error: Option<String>,
    pub filters: SearchFilters,
    pub library_sort: LibrarySort,
    /// Whether Flatpak apps update daily in the background.
    pub automatic_updates: bool,
    /// Whether enzimOS's background updater, which reads that setting, is
    /// installed; without it the setting is not offered.
    pub automatic_updater: bool,
    /// Why the last change of `automatic_updates` was not saved.
    pub automatic_updates_error: Option<String>,
    /// Which segment / order drives the Discover landing list (Cluster A).
    pub catalog_sort: model::CatalogSort,
    /// Whether the Library's activity-history panel is open. Closed by
    /// default; running work is listed at the top of the Library itself.
    pub library_activity_open: bool,

    // Framework geometry cells update without reducer messages; layout measurements never feed back into list height.
    pub library_scroll: enzui_widgets::scroll_view::ScrollGeometryCell,
    pub search_scroll: enzui_widgets::scroll_view::ScrollGeometryCell,
    pub category_scroll: enzui_widgets::scroll_view::ScrollGeometryCell,

    // Lazy initialization supports views before the first reducer message; input changes invalidate the projection.
    pub(crate) projection: std::cell::OnceCell<projection::RowProjection>,
    pub(crate) projection_builds: std::cell::Cell<u64>,
    pub(crate) adopted_rows: std::cell::OnceCell<projection::RowProjection>,

    // Progress-only changes retain this subtree; installed rows, sorting and filtering invalidate it.
    pub(crate) library_cache: SubtreeHandle,
    /// The running work the Library's cached rows were built with.
    pub(crate) library_operations_signature: u64,

    // The scroll offset the retained Library rows were last realised at.
    //
    // Written by the cached-subtree builder, read by the scroll reducer. The
    // live scroll geometry cannot answer that question: the scrollable stamps
    // its new translation during the same event pass that publishes the
    // scroll message, so a reducer comparing against it would see the move as
    // already applied and skip the invalidation the retained rows need.
    pub(crate) library_realised_at: std::rc::Rc<std::cell::Cell<f32>>,

    // ops
    pub operations: Vec<OperationState>,
    // Cancellation handles use attempt identities so a stale row cannot cancel a newer run.
    pub(crate) operation_cancels:
        std::collections::HashMap<model::OperationId, services::OperationCancellation>,
    // Terminal custody is independent of the progress consumer, so losing the
    // iced stream cannot strand an admitted attempt in the reducer.
    pub(crate) operation_completions:
        std::collections::HashMap<model::OperationId, OperationCompletion>,

    pub(crate) operation_attempts: std::collections::HashMap<String, u64>,
    pub(crate) activity_center: ActivityCenter,
    pub(crate) shell_activity: crate::shell_activity::ShellActivity,
    pub(crate) activity_schedule: ActivitySchedule,
    pub(crate) activity_clock: std::time::Instant,
    pub(crate) next_activity_id: u64,
    pub(crate) active_activity_keys: std::collections::HashMap<model::OperationId, ActivityKey>,
    pub(crate) pending_uninstall: Option<(String, String)>,
    pub(crate) confirm_state: enzui_core::confirm::ConfirmState,
    pub(crate) pending_remote_removal: Option<String>,
    pub(crate) remote_confirm_state: enzui_core::confirm::ConfirmState,

    // status
    pub loading_installed: bool,
    pub loading_catalog: bool,
    pub last_error: Option<String>,
    pub(crate) launching_app: Option<(String, u64)>,
    /// The last external-URL open that failed, as a one-line human
    /// message. `None` when the last publisher-link click succeeded (or
    /// when nothing has been clicked). Rendered inline in the detail-page
    /// Links row so the user sees a typed portal outcome instead of a
    /// silently-inert button. Cleared on a successful subsequent open,
    /// and when the detail page changes app.
    pub last_url_error: Option<String>,

    // seams
    // The publisher-link launcher. Production wires a lazily-constructed
    // D-Bus OpenURI backend; tests inject a
    // [`url_launch::tests::SharedMockBackend`] via
    // [`url_launch::UrlLauncher::with_backend`] so the reducer arms can
    // be driven without a session bus or a browser on the host. Owned
    // (not `Arc`'d) because the reducer is the sole owner of the exact
    // live request generation — supersession, drop, and settle all run
    // through this one field.
    pub(crate) url_launcher: url_launch::UrlLauncher,
    // Current window used for portal parent liveness.
    pub(crate) window: Option<iced::window::Id>,
    pub(crate) chrome: enzui_widgets::app_shell::ShellWindowState,
    pub(crate) closing: bool,
    pub(crate) closing_window: Option<iced::window::Id>,
    pub(crate) close_drain_session: Option<CloseDrainSession>,
    pub(crate) close_retained_owners: Option<CloseOwners>,
    pub(crate) close_drain_probe: Option<CloseDrainProbe>,
    pub(crate) close_drain_report: Option<CloseDrainReport>,
    pub(crate) close_operation_ids: Vec<model::OperationId>,
    pub(crate) close_helper_status: CloseHelperStatus,
    pub(crate) close_started: Option<std::time::Instant>,
    pub(crate) close_emitted: bool,
    pub(crate) close_keep_open_requested: bool,
    pub(crate) close_recovery_error: Option<&'static str>,

    // Bottom-right toasts: operation failures and the close-recovery failure.
    // Errors stay until dismissed.
    pub(crate) toasts: enzui_widgets::toast_host::ToastHost,
    pub(crate) toast_expiry: enzui_widgets::toast_host::ToastExpirySchedule,
    // Failed operations already raised as a toast: (app id, attempt, error, toast).
    pub(crate) toasted_failures: Vec<(String, u64, String, enzui_widgets::toast_host::ToastId)>,
    // The close-recovery failure raised as a toast, withdrawn once it clears.
    pub(crate) close_recovery_toast: Option<(&'static str, enzui_widgets::toast_host::ToastId)>,
    // The "Store is still open" text whose notice the user dismissed; a new
    // close failure shows again.
    pub(crate) close_failure_dismissed: Option<String>,

    // Injected catalogue transport keeps network I/O off the reducer and makes tests independent of Flathub.
    pub(crate) catalog_source: std::sync::Arc<dyn catalog::CatalogSource>,

    // Override writes settle through checked result messages so optimistic state can roll back.
    pub(crate) permission_override_runner: services::PermissionOverrideRunnerHandle,
    // Test defaults use a nonexistent executable to prevent unintended Flatpak mutations.
    pub(crate) flatpak_program: std::path::PathBuf,

    // theme
    pub(crate) themes: ThemeIntegration,

    // The path-free reason the store's durable roots could not be established,
    // or `None` when the roots were captured at boot. While set, every
    // catalog/icon/screenshot/recent-search read and write task is refused and
    // the store stays disabled across navigation, retry, and close until an
    // explicit reacquire succeeds.
    pub(crate) capabilities: roots::StoreCapabilities,

    // The application-owned bounded pool for Wayland parent exports. A full
    // pool settles a submission as a typed refusal, surfaced as a visible URL
    // error rather than a silent unparented open or an unbounded thread.
    pub(crate) export_owner: StoreExportOwner,
    // The live parent-export request generation. Each dispatched export
    // advances it; a settlement naming an older generation is discarded, so a
    // superseded export never adopts onto a newer request.
    pub(crate) parent_export_generation: u64,
    // The one application-owned bounded pool for off-thread catalogue reads
    // (cache load and network ingest). A saturated queue settles a submission
    // as Busy, surfaced as a retryable state rather than an unbounded thread.
    pub(crate) catalog_owner: enzui_core::blocking_task::BlockingTaskOwner,
    // The live catalogue request generation. Boot and Refresh advance it; a
    // cache or ingest settlement naming an older generation is discarded, so a
    // slow boot read cannot overwrite a newer refresh.
    pub(crate) catalog_generation: u64,
    // The bounded owner for the three detail routes. Selection cancellation
    // reaches every closure, while shutdown prevents new admission and bounds
    // the final join independently of the UI executor.
    pub(crate) detail_owner: enzui_core::blocking_task::BlockingTaskOwner,
    // The one bounded pool every other routed short flatpak read shares; a
    // saturated active set plus queue settles a submission as a typed refusal.
    pub(crate) read_owner: enzui_core::blocking_task::BlockingTaskOwner,
    pub(crate) next_owned_read_request: u64,
    pub(crate) deferred_owned_reads: DeferredOwnedReads,
    // Four fixed workers reserve mutation process and reaper capacity before
    // any accepted closure can spawn its Flatpak transport.
    pub(crate) mutation_owner: enzui_core::blocking_task::BlockingTaskOwner,
    // Set when the read pool refuses a recent-search reload; surfaced as a note
    // in the recovery strip and cleared on retry or settlement.
    pub(crate) recent_searches_read_busy: bool,
}
