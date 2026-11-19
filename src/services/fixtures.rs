/// Return whether `ENZUI_FIXTURE` is exactly "1"; every other value disables fixtures.
/// Production resolves this once at boot and passes the flag to downstream readers.
pub(crate) fn fixture_enabled() -> bool {
    fixture_enabled_from(std::env::var("ENZUI_FIXTURE").ok().as_deref())
}

// Parse an injected value without mutating process-wide environment.
#[must_use]
pub(crate) fn fixture_enabled_from(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Select an offline capture state independently of route selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FixtureOffline {
    /// Normal: a current fixture catalogue.
    #[default]
    Online,
    /// A cached catalogue with the network down — the stale banner.
    Stale,
    /// No cache and the network down — the actionable error page.
    Cold,
}

// Unrecognized offline modes retain normal fixture loading.
pub(crate) fn fixture_offline_mode(value: Option<&str>) -> FixtureOffline {
    match value {
        Some("stale") => FixtureOffline::Stale,
        Some("cold") => FixtureOffline::Cold,
        _ => FixtureOffline::Online,
    }
}

/// [`fixture_offline_mode`] read off the environment. Only consulted inside the
/// [`fixture_enabled`] branch of the catalogue load, so it cannot affect a real
/// user's app: without `ENZUI_FIXTURE=1` there is no fixture path to divert.
pub(crate) fn fixture_offline() -> FixtureOffline {
    fixture_offline_mode(std::env::var("ENZUI_FIXTURE_OFFLINE").ok().as_deref())
}

/// Deterministic installed rows with mixed scopes so captures include both editable and managed installations.
pub(crate) fn fixture_installed() -> Vec<AppEntry> {
    vec![
        AppEntry {
            id: "org.gnome.Weather".into(),
            name: "Weather".into(),
            version: "45.0".into(),
            branch: "stable".into(),
            origin: "flathub".into(),
            size: "200 MB".into(),
            installed: true,
            // The one row `fixture_updates()` lists, kept user-scope so the
            // update path stays exercised end to end.
            install_scope: InstallScope::User,
            ..Default::default()
        },
        AppEntry {
            id: "org.gimp.GIMP".into(),
            name: "GIMP".into(),
            version: "2.10.36".into(),
            branch: "stable".into(),

            origin: "flathub-beta".into(),
            size: "300 MB".into(),
            installed: true,
            install_scope: InstallScope::System,
            ..Default::default()
        },
        AppEntry {
            id: "org.videolan.VLC".into(),
            name: "VLC".into(),
            version: "3.0.20".into(),
            branch: "stable".into(),
            origin: "flathub".into(),
            size: "42 MB".into(),
            installed: true,
            install_scope: InstallScope::User,
            ..Default::default()
        },
    ]
}


// Keep example repositories public and isolated from host configuration.
pub(crate) fn fixture_remotes() -> Vec<RemoteInfo> {
    vec![
        // The catalogue fixture needs a user-scope source to exercise installable routes end to end.
        RemoteInfo {
            name: "flathub".into(),
            title: "Flathub".into(),
            url: "https://dl.flathub.org/repo/".into(),
            options: "user".into(),
        },
        // Match the system-installed fixture app to this read-only repository.
        RemoteInfo {
            name: "flathub-beta".into(),
            title: "Flathub Beta".into(),
            url: "https://dl.flathub.org/beta-repo/".into(),
            options: "system".into(),
        },
    ]
}

// Additional activity is opt-in, independent of the default fixture population.
pub(crate) fn fixture_operations() -> Vec<crate::model::OperationState> {
    vec![
        crate::model::OperationState {
            attempt: 1,
            cancel: crate::model::CancelState::Idle,
            app_id: "org.gnome.Maps".into(),
            name: "Maps".into(),
            kind: crate::model::OpKind::Install,
            progress: 0.35,
            sampled: true,
            message: "Installing…".into(),
            finished: false,
            error: None,
        },
        crate::model::OperationState {
            attempt: 1,
            cancel: crate::model::CancelState::Idle,
            app_id: "org.gnome.Calendar".into(),
            name: "Calendar".into(),
            kind: crate::model::OpKind::Update,
            progress: 0.72,
            sampled: true,
            message: "Updating…".into(),
            finished: false,
            error: None,
        },
        crate::model::OperationState {
            attempt: 1,
            cancel: crate::model::CancelState::Idle,
            app_id: "org.gnome.Calculator".into(),
            name: "Calculator".into(),
            kind: crate::model::OpKind::Install,
            progress: 1.0,
            sampled: true,
            message: "Installed".into(),
            finished: true,
            error: None,
        },
        crate::model::OperationState {
            attempt: 1,
            cancel: crate::model::CancelState::Idle,
            app_id: "org.gnome.Contacts".into(),
            name: "Contacts".into(),
            kind: crate::model::OpKind::Uninstall,
            progress: 1.0,
            sampled: true,
            message: "Removed".into(),
            finished: true,
            error: None,
        },
    ]
}

pub(crate) fn fixture_recently_uninstalled() -> Vec<AppEntry> {
    vec![
        AppEntry {
            id: "org.gnome.Epiphany".into(),
            name: "GNOME Web".into(),
            // Uninstalled history has no installation scope; Reinstall targets the reviewed user scope.
            install_scope: InstallScope::Unknown,
            ..Default::default()
        },
        AppEntry {
            id: "org.gnome.Contacts".into(),
            name: "Contacts".into(),
            install_scope: InstallScope::Unknown,
            ..Default::default()
        },
    ]
}

pub(crate) fn fixture_updates() -> Vec<AvailableUpdate> {
    vec![AvailableUpdate::user("org.gnome.Weather")]
}

pub(crate) fn fixture_catalog() -> FlathubCatalogs {
    let entries = mock_catalog();
    // Deterministic slices — no `rand`, no `HashMap` iteration.
    let take = |from: usize, n: usize| -> Vec<AppEntry> {
        entries.iter().cloned().cycle().skip(from).take(n).collect()
    };
    FlathubCatalogs {
        popular: take(0, entries.len().min(8)),
        trending: take(2, entries.len().min(6)),
        new: take(5, entries.len().min(6)),
        // Start late enough that the union of all four collections covers
        // every fixture entry (including Firefox/network at index 11).
        updated: take(8, entries.len().min(6)),
    }
}

/// Fixed permissions used by detail-route captures.
pub(crate) fn fixture_permissions() -> Permissions {
    Permissions {
        shared: vec!["network".into()],
        sockets: vec!["wayland".into(), "pulseaudio".into()],
        devices: vec!["dri".into()],
        filesystems: vec!["xdg-download:ro".into()],
        session_bus: Vec::new(),
        system_bus: Vec::new(),
        features: Vec::new(),
    }
}

/// Fixed ratings histogram used by detail-route captures.
pub(crate) fn fixture_ratings() -> RatingsCounts {
    RatingsCounts {
        star1: 1,
        star2: 2,
        star3: 7,
        star4: 30,
        star5: 80,
        total: 120,
        ..Default::default()
    }
}

/// Fixed enrichment used by detail-route captures. It deliberately declares
/// no screenshots: the resolved empty state is deterministic and avoids both
/// appstream and image-download traffic.
pub(crate) fn fixture_detail_context() -> DetailContext {
    let releases = vec![
        FlathubRelease {
            version: "4.3.2".into(),
            timestamp: "2024-12-03".into(),
            description: "Stability improvements and workflow refinements.".into(),
            release_type: "stable".into(),
        },
        FlathubRelease {
            version: "4.3.1".into(),
            timestamp: "2024-11-19".into(),
            description: "Maintenance release.".into(),
            release_type: "stable".into(),
        },
    ];
    let mut content_rating = std::collections::HashMap::new();
    content_rating.insert("oars-1.1".into(), "mild".into());
    content_rating.insert("violence-cartoon".into(), "mild".into());
    DetailContext {
        metadata_state: DetailRouteState::Known,
        stats_state: DetailRouteState::Known,
        ratings_state: DetailRouteState::Known,
        license: "GPL-3.0-or-later".into(),
        license_kind: LicenseKind::Foss,
        // Default captures show answered verification; dedicated fixtures cover unknown authority.
        verification: PublisherVerification::Verified {
            method: "website".into(),
            provenance: VerificationProvenance::Live,
        },
        installs_last_month: 612_000,
        age_rating: "12+".into(),
        content_rating,
        last_updated: newest_release_date(&releases),
        releases,
        urls: FlathubUrls {
            homepage: "https://www.blender.org/".into(),
            bugtracker: "https://projects.blender.org/blender/blender/issues".into(),
            donation: "https://fund.blender.org/".into(),
            help: "https://docs.blender.org/".into(),
            ..Default::default()
        },
        download_size: 212_000_000,
        ratings: Some(fixture_ratings()),
        screenshot_count: 0,
        ..Default::default()
    }
}
