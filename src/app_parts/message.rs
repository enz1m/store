//! Store messages and their trait classifications.

use super::*;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) enum Message {
    #[cfg(test)]
    InstalledLoaded(Result<Vec<AppEntry>, String>),
    #[cfg(test)]
    UpdatesLoaded {
        generation: u64,
        scan: Box<services::UpdateScan>,
    },
    #[cfg(test)]
    CatalogLoaded(Box<catalog::CatalogLoad>),

    Theme(Box<ThemeEvent>),

    /// A later Store launch should raise its existing window.
    Launched,
    ReadOwnerRetryTick,
    // navigation
    Navigate(Route),
    Back,
    CategorySelected(String),

    OpenAppDetail(String),
    /// Open an installed app in its independently managed desktop lifetime.
    LaunchInstalled(String),
    /// Completion of one bounded launch handoff, identified by app and request.
    AppLaunched(String, u64, Result<(), String>),

    #[cfg(feature = "center")]
    Center(enzui_agent::center::ConnectorEvent),
    OpenAppDetailFrom {
        app_id: String,
        origin: String,
        install_scope: services::InstallScope,
    },
    /// The detail page's background permission-metadata read completed. Carries
    /// the app id and detail generation it was dispatched for so a completion
    /// for a page the user has left is refused before it can publish onto the
    /// current page, whether it succeeded or failed.
    PermissionsLoaded {
        app_id: String,
        generation: u64,
        result: Result<services::Permissions, String>,
    },
    /// Re-run the detail page's permission-metadata read after a failed load.
    /// Carries the app id so a retry for a page the user has left is ignored.
    PermissionsRetryRequested(String),
    /// Enrichment result bound to its requested app and detail generation.
    DetailContextLoaded {
        app_id: String,
        generation: u64,
        context: Box<services::DetailContext>,
    },

    VerificationRetryRequested(String),

    VerificationLoaded {
        app_id: String,
        generation: u64,
        verification: services::PublisherVerification,
    },
    /// One identity-bound screenshot request completed. The app id and
    /// generation prevent a late result from an older detail navigation from
    /// replacing the current app's gallery.
    ScreenshotsLoaded {
        app_id: String,
        generation: u64,
        result: services::ScreenshotFetchResult,
    },
    OpenScreenshot(usize),
    ScreenshotViewerEvent(enzui_widgets::full_screen_overlay::FullScreenOverlayEvent),
    /// Open the "Changelogs" release-history dialog (Cluster A).
    OpenReleaseHistory,
    /// Close the release-history dialog.
    CloseReleaseHistory,
    ReleaseHistoryEvent(enzui_widgets::modal_host::ModalHostEvent),
    /// Open an external publisher link (Homepage / Bug tracker / Donate /
    /// Help) via the XDG Desktop Portal OpenURI transport (see
    /// `url_launch.rs`).
    OpenUrl(String),
    /// Records a window that can own portal requests.
    WindowOpened(iced::window::Id),
    ActivityWindowTagged(iced::window::Id, String, bool),
    /// Invalidates a portal parent window.
    WindowClosed(iced::window::Id),
    /// Delivers a parent export for a queued URL open.
    OpenUrlParentExported {
        /// Target URL.
        url: String,
        /// Move-once parent settlement from the bounded export pool, carrying
        /// the export generation so a superseded settlement is refused.
        parent: enzui_parent_lease::Handoff<
            enzui_core::blocking_task::BlockingTaskSettlement<enzui_wayland::ParentAdoption>,
        >,
    },
    /// Wake edge from `UrlLauncher::completion_subscription` — the portal's
    /// own event edge, not a clock. Carries the launcher request
    /// generation that was live when the subscription was installed, so a
    /// stale queued wake from a superseded generation is rejected before
    /// pumping the current request. Present only when a request is
    /// actually pending, so a settled app holds no wake source at all.
    OpenUrlPortalTick(u64),
    /// The single completion arm for a publisher-link open. Every
    /// [`url_launch::UrlLaunchOutcome`] variant reaches this arm — both a
    /// late portal reply and a request that never started — so
    /// `last_url_error` reflects one typed outcome per attempt.
    OpenUrlFinished(url_launch::UrlLaunchOutcome),
    OpenPermissionsDialog,
    ClosePermissionsDialog,
    PermissionsDialogEvent(enzui_widgets::modal_host::ModalHostEvent),

    ToggleRatingsBreakdown,
    // Cluster B: the "Sandbox Permissions" editing dialog.
    /// Open the editing dialog and load the app's tri-state toggle set.
    OpenSandboxDialog,
    /// A closed dialog's host, which presents nothing and so reports nothing.
    DialogHostIdle,

    OwnedPermissionTogglesLoaded {
        request: u64,
        app_id: String,
        generation: u64,
        outcome: services::PermissionTogglesOutcome,
    },
    PermissionTogglesReadTerminated {
        request: u64,
        app_id: String,
        generation: u64,
        terminal: flatpak::ReadTaskError,
    },
    /// Re-run the Sandbox Permissions build after an unavailable outcome,
    /// without re-opening the dialog or disturbing its focus.
    SandboxPermissionsRetry,
    /// Flip one permission on/off (by machine key), applying the override.
    TogglePermission(String, bool),
    /// Checked permission-write completion bound to its app and detail generation.
    OwnedPermissionMutationFinished {
        app_id: String,
        generation: u64,
        completion: Box<Message>,
    },
    PermissionOverrideFinished {
        key: String,
        attempted: bool,
        previous: bool,
        result: Result<(), services::PermissionActionFailure>,
    },
    /// Reset **every** permission to the app's declared defaults.
    ResetPermissionsToDefaults,
    /// Completion of a checked whole-policy reset. The previous list is kept
    /// so a failed reset restores every optimistic change, not only one row.
    PermissionResetFinished {
        previous: Vec<services::PermissionToggle>,
        result: Result<(), services::PermissionActionFailure>,
    },
    /// Close the editing dialog.
    CloseSandboxDialog,
    SandboxDialogEvent(enzui_widgets::modal_host::ModalHostEvent),
    SandboxPolicyDetails(bool),

    // search
    SearchInput(String),
    SearchCleared,
    /// The Library-pane filter query changed — filters the installed list.
    LibrarySearchInput(String),
    /// The Library-pane filter was cleared.
    LibrarySearchCleared,
    /// The user pressed Enter in the search field — commit the current query
    /// to the persisted recent-search history (Cluster A).
    SearchSubmitted,
    /// The user clicked a recent-search chip — re-run that query.
    RecentSearchPicked(String),
    /// The user asked to re-read a refused recent-search history; the reread
    /// runs in a generation-bound task (Cluster A recovery).
    RecentSearchesRetry,
    /// A generation-bound recent-search reread worker finished; its result waits
    /// in the reload slot. A stale generation installs neither history nor
    /// authority.
    RecentSearchesReloaded {
        /// The reload epoch this reread was dispatched under.
        generation: u64,
    },
    /// The bounded read pool refused the recent-search reload before it reached
    /// the disk. `busy` distinguishes a saturated-queue refusal from a pool
    /// shutdown; both leave the retry available.
    RecentSearchesReloadRefused {
        /// The reload epoch this refused submission was dispatched under.
        generation: u64,
        /// Whether the refusal was a saturated-queue admission refusal.
        busy: bool,
    },
    /// The user chose to retire a refused recent-search history, replacing it
    /// with the current in-memory list through an identity-bound publication.
    RecentSearchesRetired,

    ToggleVerifiedOnly,
    ToggleFossOnly,
    ToggleHideEol,
    ToggleInstalledOnly,

    // discover — the segment / sort order of the landing list (Cluster A)
    /// Expand an uncapped Discover collection; Back returns to sectioned Discover.
    ExpandCollection(model::CollectionKind),

    /// Stable app IDs preserve the selected carousel item across catalogue reorderings.
    FeaturedPageSelected(enzui_core::paging::PageId),
    /// A hero load finished on the read pool. Honoured only while `generation`
    /// is the app's live attempt and the app is still a current hero page, so a
    /// superseded or retired settlement is discarded.
    FeaturedImageLoaded {
        generation: u64,
        outcome: featured_image::HeroImageOutcome,
    },
    /// The read pool declined a hero load before it ran. A `busy` saturation is
    /// retained for a later retry tick; otherwise the owner is gone and the
    /// hero settles unavailable. Applied only to the app's live `generation`.
    FeaturedImageRefused {
        app_id: String,
        generation: u64,
        busy: bool,
    },
    /// Coalesced wake that re-drives one retained hero intent while any remain
    /// pending.
    FeaturedRetryTick,

    // library
    LibrarySortChanged(LibrarySort),
    /// Open or close the Library's sort menu.
    LibrarySortMenu(bool),
    AutomaticUpdatesToggled(bool),
    AutomaticUpdatesSaved(bool, Result<(), String>),
    /// Publish the installed-list offset so its cached virtual rows are
    /// rebuilt for the scrollable's current window.
    LibraryScrolled(f32),
    // Deterministic test geometry exercises the same scroll offsets published by the widget.

    // data loads
    RefreshRequested,

    OwnedInstalledLoaded {
        request: u64,
        result: Result<Vec<AppEntry>, String>,
    },
    InstalledReadTerminated {
        request: u64,
        terminal: flatpak::ReadTaskError,
    },

    OwnedUpdatesLoaded {
        request: u64,
        generation: u64,
        scan: Box<services::UpdateScan>,
    },
    UpdatesReadTerminated {
        request: u64,
        generation: u64,
        terminal: flatpak::ReadTaskError,
    },

    UpdateCheckRetryRequested,

    OwnedRemotesLoaded {
        request: u64,
        result: Result<Vec<services::RemoteInfo>, String>,
    },
    RemotesReadTerminated {
        request: u64,
        terminal: flatpak::ReadTaskError,
    },
    EnzimosCatalogueLoaded {
        request: u64,
        apps: Option<Vec<AppEntry>>,
    },

    /// The persisted cache settled off-thread on the catalogue pool, carrying
    /// the request generation it was dispatched under so a slow boot read
    /// cannot repaint over a newer refresh. A warm cache paints here before the
    /// network ingest runs.
    CatalogCacheLoaded {
        generation: u64,
        phase: Box<CatalogCachePhase>,
    },
    /// A network ingest settled off-thread, carrying its request generation.
    /// Discarded when a newer refresh has since advanced the generation.
    CatalogIngested {
        generation: u64,
        load: Box<catalog::CatalogLoad>,
    },
    /// The catalogue pool refused a submission: `busy` when its bounded queue
    /// was full, otherwise the pool is shut down. Carries the generation so a
    /// stale refusal cannot disturb a newer request.
    CatalogUnavailable {
        generation: u64,
        busy: bool,
    },

    // operations — every one of these is reachable only from a button press
    InstallRequested(String, String),
    InstallReviewLoaded(u64, Result<serde_json::Value, String>),
    InstallConsentHost(enzui_widgets::modal_host::ModalHostEvent),
    InstallConsentCancel,
    InstallConsentConfirm,
    InstallConsentBlock(usize, bool),
    InstallConsentDetails,
    UpdateRequested(String, String),
    UpdateAllRequested,
    UninstallConfirmRequested(String, String),
    UninstallConfirmEvent(ConfirmDialogEvent),
    OperationEvent(flatpak::OpEvent),
    OperationPoll,
    // Activity and row cancellation share the same attempt-specific reducer path.
    /// Cancel the exact operation attempt; a stale render cannot cancel its replacement.
    CancelOperationRequested(model::OperationId),
    ActivityPrune(u64),

    ClearFinishedTasks,
    /// Remove failed operations from the Library's in-progress section.
    DismissFailedTasks,
    /// Open or close the Library's activity-history panel.
    LibraryActivityToggled,
    ClearRecentlyUninstalled,

    // remotes (cluster 1). The add-remote form is a modal over the whole page;
    // its whole event stream (Escape / scrim / Tab) is routed through
    // `AddRemoteDialogEvent`, and the two fields are edited through
    // `AddRemoteNameChanged` / `AddRemoteUrlChanged`. `AddRemoteSubmit`
    // validates in `model::validate_remote` and only then shells out to
    // `services::add_remote`; `RemoteAdded` lands the result.
    AddRemoteOpened,
    AddRemoteNameChanged(String),
    AddRemoteUrlChanged(String),
    AddRemoteSubmit,
    AddRemoteCancelled,
    AddRemoteDialogEvent(enzui_widgets::modal_host::ModalHostEvent),
    /// The titlebar's Repositories button.
    OpenRepositoriesDialog,
    /// The Repositories dialog's Close.
    CloseRepositoriesDialog,
    /// Escape, a scrim press or Tab in the Repositories dialog.
    RepositoriesDialogEvent(enzui_widgets::modal_host::ModalHostEvent),
    /// Completion for one exact Add Remote showing. A stale completion from a
    /// closed/reopened form is ignored even when its draft happens to match.
    RemoteAdded(u64, Result<(), String>),
    /// Pre-mutation repository read bound to its draft generation.
    RemoteAddPlanned(u64, Result<services::RemoteAddPlan, String>),

    AddRemoteReplaceConfirmed,
    /// Return a repository conflict to name editing without running Flatpak.
    AddRemoteRenameRequested,
    /// Replacement result containing the repository URL read back after mutation.
    RemoteReplaced(u64, Result<String, String>),
    /// Request removal of a user repository. This only arms the confirmation;
    /// `RemoveRemoteConfirmEvent` is the sole path that starts `remote-delete`.
    RemoveRemoteRequested(String),
    RemoveRemoteConfirmEvent(ConfirmDialogEvent),
    /// A remote removal resolved; carries the subject so success can remove
    /// exactly that row before the refreshed backend list arrives.
    RemoteRemoved(String, Result<(), String>),

    // Modal hosts own traversal; otherwise the shell uses the framework wrapping operation.
    FocusNext,

    FocusPrevious,

    /// Ctrl+F: open the search popup with the caret in its field.
    FocusSearch,
    /// The titlebar search icon: open or close the search popup.
    SearchPopupToggled,
    /// An outside press or Escape closed the search popup.
    SearchPopupDismissed,
    /// The search popup's query changed; it searches without leaving the page.
    SearchPopupInput(String),
    /// "Show all results", or Enter: the Search page for the query.
    SearchPopupShowAll,
    /// Icons fetched for rows a list shows (search popup, Search, a category).
    RowIconsLoaded(Vec<(String, crate::bounded_image::RemoteImage)>),
    /// The Search page's list scrolled to this offset.
    SearchScrolled(f32),
    /// An open category's list scrolled to this offset.
    CategoryScrolled(f32),

    // Resolve framework window intents against the host window; Close enters bounded-owner cleanup.
    /// Set the framework titlebar command menu visibility.
    TitlebarOverflow(bool),
    WindowGesture(enzui_widgets::title_bar_gesture::WindowCommand),
    WindowState(enzui_widgets::app_shell::ShellWindowEvent),
    CloseRequested(iced::window::Id),
    CloseRetry,
    CloseKeepOpen,
    CloseDrainPoll,
    /// A bottom-right toast was dismissed, acted on, hovered or is due.
    Toast(enzui_widgets::toast_host::ToastEvent),
    /// Hide the "Store is still open" notice until the close failure changes.
    DismissCloseFailure,
}

impl Message {
    pub(crate) fn open_app_detail(entry: &AppEntry) -> Self {
        Self::OpenAppDetailFrom {
            app_id: entry.id.clone(),
            origin: entry.origin.clone(),
            install_scope: entry.install_scope.clone(),
        }
    }
}

impl enzui_core::crash_report::MessageKind for Message {
    fn message_kind(&self) -> &'static str {
        match self {
            #[cfg(test)]
            Self::InstalledLoaded(..) => "InstalledLoaded",
            #[cfg(test)]
            Self::UpdatesLoaded { .. } => "UpdatesLoaded",
            #[cfg(test)]
            Self::CatalogLoaded(..) => "CatalogLoaded",

            Self::Theme(..) => "Theme",
            Self::Launched => "Launched",

            Self::LaunchInstalled(..) => "LaunchInstalled",
            Self::AppLaunched(..) => "AppLaunched",
            Self::ReadOwnerRetryTick => "ReadOwnerRetryTick",
            Self::Navigate(..) => "Navigate",
            Self::Back => "Back",
            Self::CategorySelected(..) => "CategorySelected",
            Self::OpenAppDetail(..) | Self::OpenAppDetailFrom { .. } => "OpenAppDetail",
            #[cfg(feature = "center")]
            Self::Center(..) => "Center",
            Self::PermissionsLoaded { .. } => "PermissionsLoaded",
            Self::PermissionsRetryRequested(..) => "PermissionsRetryRequested",
            Self::DetailContextLoaded { .. } => "DetailContextLoaded",
            Self::VerificationRetryRequested(..) => "VerificationRetryRequested",
            Self::VerificationLoaded { .. } => "VerificationLoaded",
            Self::ScreenshotsLoaded { .. } => "ScreenshotsLoaded",
            Self::OpenScreenshot(..) => "OpenScreenshot",
            Self::ScreenshotViewerEvent(..) => "ScreenshotViewerEvent",
            Self::OpenReleaseHistory => "OpenReleaseHistory",
            Self::CloseReleaseHistory => "CloseReleaseHistory",
            Self::ReleaseHistoryEvent(..) => "ReleaseHistoryEvent",
            Self::WindowOpened(..) => "WindowOpened",
            Self::ActivityWindowTagged(..) => "ActivityWindowTagged",
            Self::WindowClosed(..) => "WindowClosed",
            Self::OpenUrl(..) => "OpenUrl",
            Self::OpenUrlParentExported { .. } => "OpenUrlParentExported",
            Self::OpenUrlPortalTick(..) => "OpenUrlPortalTick",
            Self::OpenUrlFinished(..) => "OpenUrlFinished",
            Self::OpenPermissionsDialog => "OpenPermissionsDialog",
            Self::ClosePermissionsDialog => "ClosePermissionsDialog",
            Self::PermissionsDialogEvent(..) => "PermissionsDialogEvent",
            Self::ToggleRatingsBreakdown => "ToggleRatingsBreakdown",
            Self::OpenSandboxDialog => "OpenSandboxDialog",
            Self::DialogHostIdle => "DialogHostIdle",

            Self::OwnedPermissionTogglesLoaded { .. } => "PermissionTogglesLoaded",
            Self::PermissionTogglesReadTerminated { .. } => "PermissionTogglesReadTerminated",
            Self::SandboxPermissionsRetry => "SandboxPermissionsRetry",
            Self::TogglePermission(..) => "TogglePermission",
            Self::PermissionOverrideFinished { .. } => "PermissionOverrideFinished",
            Self::OwnedPermissionMutationFinished { .. } => "OwnedPermissionMutationFinished",
            Self::ResetPermissionsToDefaults => "ResetPermissionsToDefaults",
            Self::PermissionResetFinished { .. } => "PermissionResetFinished",
            Self::CloseSandboxDialog => "CloseSandboxDialog",
            Self::SandboxDialogEvent(..) => "SandboxDialogEvent",
            Self::SandboxPolicyDetails(..) => "SandboxPolicyDetails",
            Self::SearchInput(..) => "SearchInput",
            Self::SearchCleared => "SearchCleared",
            Self::LibrarySearchInput(..) => "LibrarySearchInput",
            Self::LibrarySearchCleared => "LibrarySearchCleared",
            Self::SearchSubmitted => "SearchSubmitted",
            Self::RecentSearchPicked(..) => "RecentSearchPicked",
            Self::RecentSearchesRetry => "RecentSearchesRetry",
            Self::RecentSearchesReloaded { .. } => "RecentSearchesReloaded",
            Self::RecentSearchesReloadRefused { .. } => "RecentSearchesReloadRefused",
            Self::RecentSearchesRetired => "RecentSearchesRetired",

            Self::ToggleVerifiedOnly => "ToggleVerifiedOnly",
            Self::ToggleFossOnly => "ToggleFossOnly",
            Self::ToggleHideEol => "ToggleHideEol",
            Self::ToggleInstalledOnly => "ToggleInstalledOnly",

            Self::ExpandCollection(..) => "ExpandCollection",
            Self::FeaturedPageSelected(..) => "FeaturedPageSelected",
            Self::FeaturedImageLoaded { .. } => "FeaturedImageLoaded",
            Self::FeaturedImageRefused { .. } => "FeaturedImageRefused",
            Self::FeaturedRetryTick => "FeaturedRetryTick",
            Self::LibrarySortChanged(..) => "LibrarySortChanged",
            Self::LibrarySortMenu(..) => "LibrarySortMenu",
            Self::AutomaticUpdatesToggled(..) => "AutomaticUpdatesToggled",
            Self::AutomaticUpdatesSaved(..) => "AutomaticUpdatesSaved",
            Self::LibraryScrolled(..) => "LibraryScrolled",

            Self::RefreshRequested => "RefreshRequested",

            Self::OwnedInstalledLoaded { .. } => "InstalledLoaded",
            Self::InstalledReadTerminated { .. } => "InstalledReadTerminated",

            Self::OwnedUpdatesLoaded { .. } => "UpdatesLoaded",
            Self::UpdatesReadTerminated { .. } => "UpdatesReadTerminated",
            Self::UpdateCheckRetryRequested => "UpdateCheckRetryRequested",

            Self::OwnedRemotesLoaded { .. } => "RemotesLoaded",
            Self::EnzimosCatalogueLoaded { .. } => "EnzimosCatalogueLoaded",
            Self::RemotesReadTerminated { .. } => "RemotesReadTerminated",

            Self::CatalogCacheLoaded { .. } => "CatalogCacheLoaded",
            Self::CatalogIngested { .. } => "CatalogIngested",
            Self::CatalogUnavailable { .. } => "CatalogUnavailable",
            Self::InstallRequested(..) => "InstallRequested",
            Self::InstallReviewLoaded(..) => "InstallReviewLoaded",
            Self::InstallConsentHost(..) => "InstallConsentHost",
            Self::InstallConsentCancel => "InstallConsentCancel",
            Self::InstallConsentConfirm => "InstallConsentConfirm",
            Self::InstallConsentBlock(..) => "InstallConsentBlock",
            Self::InstallConsentDetails => "InstallConsentDetails",
            Self::UpdateRequested(..) => "UpdateRequested",
            Self::UpdateAllRequested => "UpdateAllRequested",
            Self::UninstallConfirmRequested(..) => "UninstallConfirmRequested",
            Self::UninstallConfirmEvent(..) => "UninstallConfirmEvent",
            Self::OperationEvent(..) => "OperationEvent",
            Self::OperationPoll => "OperationPoll",

            Self::CancelOperationRequested(..) => "CancelOperationRequested",
            Self::ActivityPrune(..) => "ActivityPrune",

            Self::ClearFinishedTasks => "ClearFinishedTasks",
            Self::DismissFailedTasks => "DismissFailedTasks",
            Self::LibraryActivityToggled => "LibraryActivityToggled",
            Self::ClearRecentlyUninstalled => "ClearRecentlyUninstalled",
            Self::AddRemoteOpened => "AddRemoteOpened",
            Self::AddRemoteNameChanged(..) => "AddRemoteNameChanged",
            Self::AddRemoteUrlChanged(..) => "AddRemoteUrlChanged",
            Self::AddRemoteSubmit => "AddRemoteSubmit",
            Self::AddRemoteCancelled => "AddRemoteCancelled",
            Self::AddRemoteDialogEvent(..) => "AddRemoteDialogEvent",
            Self::OpenRepositoriesDialog => "OpenRepositoriesDialog",
            Self::CloseRepositoriesDialog => "CloseRepositoriesDialog",
            Self::RepositoriesDialogEvent(..) => "RepositoriesDialogEvent",
            Self::RemoteAdded(..) => "RemoteAdded",
            Self::RemoteAddPlanned(..) => "RemoteAddPlanned",
            Self::AddRemoteReplaceConfirmed => "AddRemoteReplaceConfirmed",
            Self::AddRemoteRenameRequested => "AddRemoteRenameRequested",
            Self::RemoteReplaced(..) => "RemoteReplaced",
            Self::RemoveRemoteRequested(..) => "RemoveRemoteRequested",
            Self::RemoveRemoteConfirmEvent(..) => "RemoveRemoteConfirmEvent",
            Self::RemoteRemoved(..) => "RemoteRemoved",
            Self::FocusNext => "FocusNext",
            Self::FocusPrevious => "FocusPrevious",
            Self::FocusSearch => "FocusSearch",
            Self::SearchPopupToggled => "SearchPopupToggled",
            Self::SearchPopupDismissed => "SearchPopupDismissed",
            Self::SearchPopupInput(..) => "SearchPopupInput",
            Self::SearchPopupShowAll => "SearchPopupShowAll",
            Self::RowIconsLoaded(..) => "RowIconsLoaded",
            Self::SearchScrolled(..) => "SearchScrolled",
            Self::CategoryScrolled(..) => "CategoryScrolled",
            Self::TitlebarOverflow(..) => "TitlebarOverflow",
            Self::WindowGesture(..) => "WindowGesture",
            Self::WindowState(..) => "WindowState",
            Self::CloseRequested(..) => "CloseRequested",
            Self::CloseRetry => "CloseRetry",
            Self::CloseKeepOpen => "CloseKeepOpen",
            Self::CloseDrainPoll => "CloseDrainPoll",
            Self::Toast(..) => "Toast",
            Self::DismissCloseFailure => "DismissCloseFailure",
        }
    }
}
