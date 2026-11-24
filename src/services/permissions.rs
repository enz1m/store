/// The repository whose metadata describes an app that is not installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MetadataRemote {
    pub name: String,
    pub system: bool,
}

pub(crate) fn read_permissions_cancellable(
    app_id: &str,
    remote: Option<&MetadataRemote>,
    cancellation: OperationCancellation,
) -> Result<Permissions, String> {
    if fixture_enabled() {
        return Ok(fixture_permissions());
    }
    read_permissions_with_executor(
        &ProcessFlatpakExecutor::cancellable(cancellation),
        app_id,
        remote,
    )
}

pub(crate) fn read_permissions_with_executor(
    executor: &dyn FlatpakExecutor,
    app_id: &str,
    remote: Option<&MetadataRemote>,
) -> Result<Permissions, String> {
    // Prefer the installed copy (`--user` scope first, falling back to system).
    let mut user_info = crate::flatpak_args::FlatpakArgs::new("info");
    user_info.options(["--user", "--show-metadata"]);
    user_info.app_id(app_id)?;
    let mut info = crate::flatpak_args::FlatpakArgs::new("info");
    info.options(["--show-metadata"]);
    info.app_id(app_id)?;
    let mut sources = vec![user_info.finish(), info.finish()];
    if let Some(remote) = remote {
        let scope = if remote.system { "--system" } else { "--user" };
        let mut remote_info = crate::flatpak_args::FlatpakArgs::new("remote-info");
        remote_info.options([scope, "--show-metadata"]);
        remote_info.remote(&remote.name)?;
        remote_info.app_id(app_id)?;
        sources.push(remote_info.finish());
    }
    // Fall through only on a complete, explicit absence; a terminal failure
    // stops here, and one shared deadline bounds all sources together.
    let deadline = Instant::now() + PERMISSION_READ_TOTAL_DEADLINE;
    for args in &sources {
        match executor.read_metadata_step(args, deadline) {
            MetadataReadStep::Present(raw) => return Ok(parse_metadata(&raw)),
            MetadataReadStep::Absent => {}
            MetadataReadStep::Terminal(error) => return Err(error),
        }
    }
    Err(PERMISSION_METADATA_ABSENT.to_owned())
}

// Minimal keyfile parser: tracks the active `[Group]`, splits `key=value`
// lines, and stitches the parts we care about into a [`Permissions`] summary.
fn parse_metadata(raw: &str) -> Permissions {
    let mut perms = Permissions::default();
    let mut group = String::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            group = line[1..line.len() - 1].to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let list = value
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(std::borrow::ToOwned::to_owned)
            .collect::<Vec<_>>();
        match group.as_str() {
            "Context" => match key {
                "shared" => perms.shared = list,
                "sockets" => perms.sockets = list,
                "devices" => perms.devices = list,
                "filesystems" => perms.filesystems = list,
                "features" => perms.features = list,
                _ => {}
            },
            "Session Bus Policy" => {
                perms.session_bus.push(format!("{key} = {value}"));
            }
            "System Bus Policy" => {
                perms.system_bus.push(format!("{key} = {value}"));
            }
            _ => {}
        }
    }
    perms
}

// ---- Sandbox permission overrides (M59) --------------------------------
//
// Flatpak's override system lets users grant or revoke sandbox permissions
// without re-installing. The override file lives at
// `~/.local/share/flatpak/overrides/<app-id>` and uses the same keyfile
// format as the app metadata. We read the current overrides, present
// common toggles, and write changes via `flatpak override --user`.

/// A single toggleable permission in the editing UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionToggle {
    /// Machine key for the override CLI (e.g. "network", "home", "x11").
    pub key: String,
    /// Human-readable label (e.g. "Network Access", "Home Directory").
    pub label: String,
    /// Category for grouping ("network", "filesystem", "device", "socket").
    pub category: String,
    /// Whether the app's metadata declares this permission.
    pub app_default: bool,
    /// Current effective state (accounting for overrides).
    pub enabled: bool,
}

/// The common permission toggles we expose in the editing UI.
/// These are the permissions most users care about; bus access and
/// features are left to advanced tools.
const COMMON_TOGGLES: &[(&str, &str, &str)] = &[
    // (key, label, category)
    ("network", "Network Access", "network"),
    ("home", "Home Directory", "filesystem"),
    ("host", "All Files (host)", "filesystem"),
    ("wayland", "Wayland Socket", "socket"),
    ("x11", "X11 Socket", "socket"),
    ("pulseaudio", "PulseAudio (Audio)", "socket"),
    ("all", "All Devices", "device"),
];

// Build the toggle list through the cancellable production transport.
pub(crate) fn build_permission_toggles_cancellable(
    app_id: &str,
    cancellation: OperationCancellation,
) -> PermissionTogglesOutcome {
    if fixture_enabled() {
        return PermissionTogglesOutcome::Ready(build_permission_toggles_from(
            fixture_permissions(),
            OverrideState::default(),
        ));
    }
    build_permission_toggles_with_executor(
        app_id,
        &ProcessFlatpakExecutor::cancellable(cancellation),
    )
}

pub(crate) fn build_permission_toggles_with_executor(
    app_id: &str,
    executor: &dyn FlatpakExecutor,
) -> PermissionTogglesOutcome {
    // The editable model is authoritative only when both the base metadata and
    // the override state are read cleanly. If either is unknown the model is
    // unavailable, never a fabricated all-disabled set opened for editing.
    let Ok(base) = read_permissions_with_executor(executor, app_id, None) else {
        return PermissionTogglesOutcome::Unavailable;
    };
    let Ok(overrides) = read_overrides_with_executor(executor, app_id) else {
        return PermissionTogglesOutcome::Unavailable;
    };
    PermissionTogglesOutcome::Ready(build_permission_toggles_from(base, overrides))
}

/// The outcome of building the Sandbox Permissions editable model.
///
/// [`PermissionTogglesOutcome::Ready`] carries an authoritative model built
/// from a clean base-metadata and override read. If either read is unknown the
/// outcome is [`PermissionTogglesOutcome::Unavailable`], never a fabricated
/// all-disabled set, so the reducer keeps mutation controls disabled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PermissionTogglesOutcome {
    Ready(Vec<PermissionToggle>),
    ReadyPolicy {
        toggles: Vec<PermissionToggle>,
        snapshot: Box<serde_json::Value>,
    },
    Unavailable,
}

fn build_permission_toggles_from(
    base: Permissions,
    overrides: OverrideState,
) -> Vec<PermissionToggle> {
    COMMON_TOGGLES
        .iter()
        .map(|(key, label, cat)| {
            let app_default = perm_is_set_in(&base, key);
            let enabled = override_effective(&overrides, key, app_default);
            PermissionToggle {
                key: key.to_string(),
                label: label.to_string(),
                category: cat.to_string(),
                app_default,
                enabled,
            }
        })
        .collect()
}

// Check whether a permission key is set in the base app metadata.
fn perm_is_set_in(perms: &Permissions, key: &str) -> bool {
    match key {
        "network" => perms.shared.iter().any(|s| s == "network"),
        "home" => perms
            .filesystems
            .iter()
            .any(|f| f == "home" || f.starts_with("home:")),
        "host" => perms
            .filesystems
            .iter()
            .any(|f| f == "host" || f.starts_with("host:")),
        "wayland" => perms.sockets.iter().any(|s| s == "wayland"),
        "x11" => perms
            .sockets
            .iter()
            .any(|s| s == "x11" || s == "fallback-x11"),
        "pulseaudio" => perms.sockets.iter().any(|s| s == "pulseaudio"),
        "all" => perms.devices.iter().any(|d| d == "all"),
        _ => false,
    }
}

// Read the current user override state through an injected executor.
pub(crate) fn read_overrides_with_executor(
    executor: &dyn FlatpakExecutor,
    app_id: &str,
) -> Result<OverrideState, String> {
    let mut args = crate::flatpak_args::FlatpakArgs::new("override");
    args.options(["--user", "--show"]);
    args.app_id(app_id)?;
    executor
        .execute(&args.finish())
        .map(|raw| parse_override_state(&raw))
}

/// Parsed user override state. Tracks both grants and revocations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OverrideState {
    /// Permissions explicitly granted by the override.
    pub granted: Permissions,
    /// Permissions explicitly revoked by the override.
    pub revoked: Permissions,
}

/// Parse the output of `flatpak override --show --user <app-id>`.
/// The format is a keyfile with `[Context]` sections. Grants look like
/// `sockets=wayland;x11;`, revocations look like `sockets=!wayland;!x11;`.
pub(crate) fn parse_override_state(raw: &str) -> OverrideState {
    let mut state = OverrideState::default();
    let mut group = String::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            group = line[1..line.len() - 1].to_string();
            continue;
        }
        if group != "Context" {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let items: Vec<&str> = value
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        for item in items {
            if let Some(negated) = item.strip_prefix('!') {
                add_to_permissions(&mut state.revoked, key, negated);
            } else {
                add_to_permissions(&mut state.granted, key, item);
            }
        }
    }
    state
}

// Helper: add a value to the correct permission category.
fn add_to_permissions(perms: &mut Permissions, ctx_key: &str, value: &str) {
    match ctx_key {
        "shared" => perms.shared.push(value.to_owned()),
        "sockets" => perms.sockets.push(value.to_owned()),
        "devices" => perms.devices.push(value.to_owned()),
        "filesystems" => perms.filesystems.push(value.to_owned()),
        "features" => perms.features.push(value.to_owned()),
        _ => {}
    }
}

// Compute the effective state of a permission given overrides.
// The override can grant what was revoked or revoke what was granted.
fn override_effective(overrides: &OverrideState, key: &str, app_default: bool) -> bool {
    // Check if the override explicitly grants or revokes this permission.
    let granted = perm_is_set_in(&overrides.granted, key);
    let revoked = perm_is_set_in(&overrides.revoked, key);
    if granted {
        return true;
    }
    if revoked {
        return false;
    }
    app_default
}

/// Build the `flatpak override --user` command args for toggling a
/// permission on or off. The app ID is validated and protected by `--`.
#[cfg(test)]
pub(crate) fn override_command_args(
    app_id: &str,
    key: &str,
    enable: bool,
) -> Result<Vec<String>, String> {
    let option = match (key, enable) {
        ("network", true) => "--share=network",
        ("network", false) => "--unshare=network",
        ("home", true) => "--filesystem=home",
        ("home", false) => "--nofilesystem=home",
        ("host", true) => "--filesystem=host",
        ("host", false) => "--nofilesystem=host",
        ("wayland", true) => "--socket=wayland",
        ("wayland", false) => "--nosocket=wayland",
        ("x11", true) => "--socket=x11",
        ("x11", false) => "--nosocket=x11",
        ("pulseaudio", true) => "--socket=pulseaudio",
        ("pulseaudio", false) => "--nosocket=pulseaudio",
        ("all", true) => "--device=all",
        ("all", false) => "--nodevice=all",
        _ => return Err(format!("unknown permission key: {key}")),
    };
    let mut args = crate::flatpak_args::FlatpakArgs::new("override");
    args.options(["--user", option]);
    args.app_id(app_id)?;
    Ok(args.finish())
}

#[cfg(test)]
fn reset_permission_args(app_id: &str) -> Result<Vec<String>, String> {
    let mut args = crate::flatpak_args::FlatpakArgs::new("override");
    args.options(["--user", "--reset"]);
    args.app_id(app_id)?;
    Ok(args.finish())
}

#[cfg(test)]
pub(crate) fn apply_permission_override_with(
    executor: &dyn FlatpakExecutor,
    app_id: &str,
    key: &str,
    enable: bool,
) -> Result<(), String> {
    executor.execute(&override_command_args(app_id, key, enable)?)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn reset_permission_overrides_with(
    executor: &dyn FlatpakExecutor,
    app_id: &str,
) -> Result<(), String> {
    executor.execute(&reset_permission_args(app_id)?)?;
    Ok(())
}

/// Inject checked permission writes so failures reach the reducer without mutating host configuration in tests.
pub(crate) trait PermissionOverrideRunner: Send + Sync {
    fn read(
        &self,
        app_id: &str,
        _scope: &str,
        cancellation: OperationCancellation,
    ) -> PermissionTogglesOutcome {
        build_permission_toggles_cancellable(app_id, cancellation)
    }
    fn checked_policy(&self) -> bool {
        false
    }
    fn apply_cancellable(
        &self,
        app_id: &str,
        key: &str,
        enable: bool,
        _token: OperationCancellation,
    ) -> Result<(), String> {
        self.apply(app_id, key, enable)
    }
    fn reset_cancellable(&self, app_id: &str, _token: OperationCancellation) -> Result<(), String> {
        self.reset(app_id)
    }
    fn apply(&self, app_id: &str, key: &str, enable: bool) -> Result<(), String>;
    fn reset(&self, app_id: &str) -> Result<(), String>;
}

pub(crate) type PermissionOverrideRunnerHandle = std::sync::Arc<dyn PermissionOverrideRunner>;

pub(crate) fn default_permission_override_runner() -> PermissionOverrideRunnerHandle {
    std::sync::Arc::new(shared_permissions::NativePermissionRunner::default())
}

/// The `flatpak` executable a mutating operation spawns, as injectable store
/// data rather than a literal inside the task builder.
///
/// A production build hands the bare `flatpak` name to the process transport,
/// which resolves it once through executable authority. A `cfg(test)` build
/// returns an absolute path that cannot exist, so a stray spawn fails before
/// any Flatpak code runs; a test that observes a real child overrides the field
/// with its own fixture program.
#[must_use]
pub(crate) fn default_flatpak_program() -> std::path::PathBuf {
    #[cfg(test)]
    {
        std::path::PathBuf::from(NO_FLATPAK_PROGRAM)
    }
    #[cfg(not(test))]
    {
        std::path::PathBuf::from("flatpak")
    }
}

/// The deliberately absent program `cfg(test)` builds spawn instead of the
/// host's `flatpak`.
///
/// Absolute so no `PATH` entry can accidentally supply it, and named so that a
/// stray spawn error in a test log says why it happened.
#[cfg(test)]
pub(crate) const NO_FLATPAK_PROGRAM: &str =
    "/nonexistent/enzui-store-app-tests-must-not-run-flatpak";

// Restore every effective permission from its declared app default.
pub(crate) fn reset_toggles_to_defaults(toggles: &mut [PermissionToggle]) {
    for t in toggles.iter_mut() {
        t.enabled = t.app_default;
    }
}

/// A small, hand-curated Explore catalog. In production the app hits the
/// Flathub REST API (`/collection/popular` etc); we ship this so Explore is
/// non-empty even when the network is unreachable and the build environment
/// has no flatpaks installed. IDs are real Flathub apps so a manual test-drive
/// can install one and see the "Installed" badge light up.
pub(crate) fn mock_catalog() -> Vec<AppEntry> {
    let entry = |id: &str,
                 name: &str,
                 publisher: &str,
                 summary: &str,
                 verified: bool,
                 downloads: u64,
                 category: &str,
                 tags: &[&str]| AppEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        publisher: publisher.to_owned(),
        summary: summary.to_owned(),
        origin: "flathub".to_owned(),
        downloads,
        verified,
        category: category.to_owned(),
        tags: tags.iter().map(std::string::ToString::to_string).collect(),
        size: match downloads % 4 {
            0 => "42 MB",
            1 => "128 MB",
            2 => "76 MB",
            _ => "212 MB",
        }
        .to_owned(),
        description: format!(
            "{name} is distributed as a flatpak from {publisher}. It runs inside a sandbox and \
             only the permissions listed on this page are granted to it. Use the Permissions \
             button below for the full list."
        ),
        // Catalogue entries have Unknown installation scope; installed-copy authority comes from the installed list.
        ..Default::default()
    };
    vec![
        entry(
            "org.gnome.Weather",
            "Weather",
            "The GNOME Project",
            "Show weather conditions and forecasts",
            true,
            32_000,
            "utility",
            &["Free Software", "Safe", "Desktop Only"],
        ),
        entry(
            "org.gimp.GIMP",
            "GIMP",
            "GIMP Team",
            "The GNU Image Manipulation Program",
            true,
            824_000,
            "graphics",
            &["Free Software", "Verified", "Desktop"],
        ),
        entry(
            "org.blender.Blender",
            "Blender",
            "Blender Foundation",
            "Free and open source 3D creation suite",
            true,
            612_000,
            "graphics",
            &["Free Software", "Verified"],
        ),
        entry(
            "org.videolan.VLC",
            "VLC",
            "VideoLAN",
            "The open-source multimedia player",
            true,
            981_000,
            "audiovideo",
            &["Free Software", "Popular"],
        ),
        entry(
            "com.mattjakeman.ExtensionManager",
            "Extension Manager",
            "Matthew Jakeman",
            "Browse and install GNOME shell extensions",
            true,
            48_000,
            "system",
            &["Free Software", "Desktop Only"],
        ),
        entry(
            "io.bassi.Amberol",
            "Amberol",
            "Emmanuele Bassi",
            "Small and simple sound and music player",
            true,
            26_000,
            "audio",
            &["Free Software", "Verified"],
        ),
        entry(
            "org.inkscape.Inkscape",
            "Inkscape",
            "Inkscape Team",
            "Vector graphics editor",
            true,
            412_000,
            "graphics",
            &["Free Software", "Verified"],
        ),
        entry(
            "com.github.tchx84.Flatseal",
            "Flatseal",
            "Martin Abente",
            "Manage flatpak permissions",
            true,
            188_000,
            "system",
            &["Free Software", "Utility"],
        ),
        entry(
            "org.kde.krita",
            "Krita",
            "KDE",
            "Digital painting app for artists",
            true,
            521_000,
            "graphics",
            &["Free Software", "Verified"],
        ),
        entry(
            "com.usebottles.bottles",
            "Bottles",
            "Bottles Team",
            "Run Windows software and games on Linux",
            true,
            296_000,
            "game",
            &["Free Software", "Verified"],
        ),
        entry(
            "io.github.celluloid_player.Celluloid",
            "Celluloid",
            "Celluloid Team",
            "Simple GTK+ frontend for mpv",
            true,
            94_000,
            "audiovideo",
            &["Free Software"],
        ),
        entry(
            "org.mozilla.firefox",
            "Firefox",
            "Mozilla",
            "Fast, Private & Safe Web Browser",
            true,
            1_240_000,
            "network",
            &["Free Software", "Verified", "Popular"],
        ),
    ]
}

/// Catalogue-only fixtures exercise search results that appear on no editorial rail.
pub(crate) fn mock_catalog_tail() -> Vec<AppEntry> {
    let entry = |id: &str,
                 name: &str,
                 publisher: &str,
                 summary: &str,
                 verified: bool,
                 downloads: u64,
                 category: &str| AppEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        publisher: publisher.to_owned(),
        summary: summary.to_owned(),
        origin: "flathub".to_owned(),
        downloads,
        verified,
        category: category.to_owned(),
        description: format!(
            "{name} is distributed as a flatpak from {publisher}. It runs inside a sandbox and \
             only the permissions listed on this page are granted to it."
        ),
        ..Default::default()
    };
    vec![
        entry(
            "org.mozilla.thunderbird",
            "Thunderbird",
            "MZLA Technologies",
            "Free and open source email, newsfeed, chat, and calendaring client",
            true,
            28_555,
            "network",
        ),
        entry(
            "org.libreoffice.LibreOffice",
            "LibreOffice",
            "The Document Foundation",
            "The LibreOffice productivity suite",
            true,
            50_339,
            "office",
        ),
        entry(
            "com.spotify.Client",
            "Spotify",
            "Spotify",
            "Online music streaming service",
            false,
            124_102,
            "audiovideo",
        ),
        entry(
            "org.audacityteam.Audacity",
            "Audacity",
            "Muse Group",
            "Audio editing and recording app",
            false,
            21_609,
            "audiovideo",
        ),
        entry(
            "com.obsproject.Studio",
            "OBS Studio",
            "OBS Project",
            "Live stream and record videos",
            true,
            94_238,
            "audiovideo",
        ),
        entry(
            "org.kde.kdenlive",
            "Kdenlive",
            "KDE",
            "Video editor",
            true,
            37_462,
            "audiovideo",
        ),
        entry(
            "md.obsidian.Obsidian",
            "Obsidian",
            "Obsidian",
            "Markdown-based knowledge base",
            true,
            80_887,
            "office",
        ),
        entry(
            "com.discordapp.Discord",
            "Discord",
            "Discord Inc.",
            "Talk, play, hang out",
            true,
            191_245,
            "network",
        ),
        entry(
            "org.telegram.desktop",
            "Telegram",
            "Telegram FZ-LLC",
            "New era of messaging",
            true,
            92_600,
            "network",
        ),
        entry(
            "org.signal.Signal",
            "Signal Desktop",
            "Signal Foundation",
            "Private messenger",
            false,
            23_613,
            "network",
        ),
        entry(
            "com.valvesoftware.Steam",
            "Steam",
            "Valve Corporation",
            "Launcher for the Steam software distribution service",
            false,
            103_901,
            "game",
        ),
        entry(
            "net.lutris.Lutris",
            "Lutris",
            "Lutris Team",
            "Video game preservation platform",
            true,
            58_205,
            "game",
        ),
        entry(
            "org.gnome.Boxes",
            "Boxes",
            "The GNOME Project",
            "Virtualization made simple",
            true,
            18_588,
            "system",
        ),
        entry(
            "com.visualstudio.code",
            "Visual Studio Code",
            "Microsoft Corporation",
            "Code editing. Redefined.",
            false,
            58_872,
            "development",
        ),
        entry(
            "org.qbittorrent.qBittorrent",
            "qBittorrent",
            "The qBittorrent Project",
            "An open-source Bittorrent client",
            true,
            60_793,
            "network",
        ),
        entry(
            "com.github.johnfactotum.Foliate",
            "Foliate",
            "John Factotum",
            "Read e-books in style",
            true,
            13_769,
            "office",
        ),
        entry(
            "org.kde.okular",
            "Okular",
            "KDE",
            "View and annotate documents",
            true,
            14_775,
            "graphics",
        ),
        entry(
            "org.darktable.Darktable",
            "darktable",
            "darktable developers",
            "Organize and develop images from digital cameras",
            false,
            21_742,
            "graphics",
        ),
        entry(
            "org.shotcut.Shotcut",
            "Shotcut",
            "Meltytech, LLC",
            "Video editor",
            true,
            15_878,
            "audiovideo",
        ),
        entry(
            "com.calibre_ebook.calibre",
            "calibre",
            "Kovid Goyal",
            "The one stop solution to all your e-book needs",
            false,
            11_762,
            "office",
        ),
        entry(
            "org.freecad.FreeCAD",
            "FreeCAD",
            "The FreeCAD Team",
            "An open source parametric 3D CAD modeler",
            true,
            30_799,
            "graphics",
        ),
        entry(
            "org.kicad.KiCad",
            "KiCad",
            "The KiCad Developers",
            "An EDA suite for schematic and circuit board design",
            true,
            7_259,
            "development",
        ),
        entry(
            "org.wireshark.Wireshark",
            "Wireshark",
            "The Wireshark developer community",
            "Network protocol analyzer",
            false,
            4_363,
            "network",
        ),
        entry(
            "com.github.wwmm.easyeffects",
            "Easy Effects",
            "Wellington Wallace",
            "Simple audio effects",
            true,
            30_228,
            "audio",
        ),
        entry(
            "org.gnome.Fractal",
            "Fractal",
            "The Fractal Team",
            "Chat on Matrix",
            true,
            1_359,
            "network",
        ),
        entry(
            "com.transmissionbt.Transmission",
            "Transmission",
            "The Transmission Project",
            "Download and share files over BitTorrent",
            false,
            7_357,
            "network",
        ),
        entry(
            "org.gnome.Calculator",
            "Calculator",
            "The GNOME Project",
            "Perform arithmetic, scientific or financial calculations",
            true,
            12_509,
            "utility",
        ),
    ]
}

pub(crate) struct CancellablePermissionRunner<'a> {
    pub runner: &'a dyn PermissionOverrideRunner,
    pub cancellation: OperationCancellation,
}
impl PermissionOverrideRunner for CancellablePermissionRunner<'_> {
    fn apply(&self, app_id: &str, key: &str, enable: bool) -> Result<(), String> {
        self.runner
            .apply_cancellable(app_id, key, enable, self.cancellation.clone())
    }
    fn reset(&self, app_id: &str) -> Result<(), String> {
        self.runner
            .reset_cancellable(app_id, self.cancellation.clone())
    }
}
