/// Catalogue and installed-app metadata; missing cached fields retain their defaults.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct AppEntry {
    pub id: String,
    pub name: String,
    /// One-line description, matching the "Trending" cards.
    pub summary: String,
    /// The origin remote (usually "flathub"), used for the detail page and to
    /// disambiguate rows when multiple remotes host the same app id.
    pub origin: String,
    pub branch: String,
    pub version: String,
    /// Human-readable download / installed size (e.g. "184 MB"). Empty when
    /// unknown.
    pub size: String,
    /// Monthly downloads reported by the catalogue, or zero when unknown.
    pub downloads: u64,
    pub installed: bool,
    /// Live deployment identity from the local Flatpak list, never catalogue JSON.
    #[serde(skip)]
    pub deployed_arch: String,
    /// Active commit hint from the local installed read; absent fields disable local icons.
    #[serde(skip)]
    pub deployed_commit: String,
    /// Whether the app has an update pending on a remote (drives the badge in
    /// the Library and the row's update button).
    pub update_available: bool,
    /// Catalogue publisher verification does not attest an installed deployment.
    pub verified: bool,
    /// Source established by the configured, signature-checking enzim Apps remote.
    #[serde(skip)]
    pub from_enzimos: bool,
    /// Optional publisher / developer name; renders under the app title on the
    /// detail page.
    pub publisher: String,
    /// Optional long description (paragraphs) for the detail page.
    pub description: String,
    /// Bazaar-style "stat pills" — safe / verified / etc. See
    /// [`Permissions::stat_pills`] for the runtime version.
    pub tags: Vec<String>,
    /// Icon URL from Flathub API. Empty when unknown.
    pub icon_url: String,
    /// Validated RGBA for a remote icon. Skipped in the catalogue JSON and
    /// rehydrated through the bounded warm-cache decoder on load.
    #[serde(skip)]
    pub icon_image: Option<crate::bounded_image::RenderImage>,
    #[serde(skip)]
    pub enzimos_detail: Option<std::sync::Arc<DetailContext>>,
    /// SPDX license expression (e.g. `"GPL-3.0-or-later"`, `"Proprietary"`).
    /// Populated for detail-page apps (from the appstream endpoint) and left
    /// empty on collection hits — the tile shows "—" until the per-app fetch
    /// resolves.
    pub license: String,
    /// Unknown catalogue support status defaults to false.
    pub is_eol: bool,
    /// Lowercase Freedesktop main category, empty when unknown.
    pub category: String,
    /// The authoritative installation scope used to authorize mutation; uninstalled catalogue entries stay Unknown.
    pub install_scope: InstallScope,
    /// Projected duplicate installations in stable read order; authorization uses the raw installed list instead.
    pub also_installed_in: Vec<InstallScope>,
    /// Source observed when this app was removed; set only on recently-removed rows, never cached.
    #[serde(skip)]
    pub removed_origin: Option<RemovedOrigin>,
}

impl AppEntry {
    pub(crate) fn has_verified_badge(&self) -> bool {
        self.verified || self.from_enzimos
    }

    /// Whether removal leaves an installation this build cannot modify.
    #[must_use]
    pub(crate) fn system_copy_remains(&self) -> bool {
        self.also_installed_in
            .iter()
            .any(InstallScope::is_system_managed)
    }
}

/// The repository a removed app came from, as flatpak reported it at removal.
///
/// A recently-removed app that no catalogue lists has no other record of its
/// source. This keeps the positively observed remote so a reinstall can be
/// authorized, but only while the same-named remote still exists in the same
/// installation with the same URL; see [`removed_origin_authorization`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct RemovedOrigin {
    /// The `origin` remote name from the installed-app read.
    pub remote: String,
    /// The remote's URL from the repositories read at removal.
    pub url: String,
    /// The installation that held both the app and the remote.
    pub scope: InstallScope,
    /// Deployed branch, when the installed read reported one.
    pub branch: String,
    /// Deployed architecture, when the installed read reported one.
    pub arch: String,
}

impl RemovedOrigin {
    /// Record the source of an installed row about to leave the Library.
    ///
    /// `None` unless a configured remote in the row's own installation carries
    /// the row's origin name and a URL; an unobserved source is not recorded.
    #[must_use]
    pub(crate) fn observe(entry: &AppEntry, remotes: &[RemoteInfo]) -> Option<Self> {
        let remote = entry.origin.trim();
        if remote.is_empty()
            || !matches!(
                entry.install_scope,
                InstallScope::User | InstallScope::System
            )
        {
            return None;
        }
        let observed = remotes.iter().find(|candidate| {
            candidate.name == remote
                && InstallScope::from_remote_options(&candidate.options) == entry.install_scope
                && !candidate.url.trim().is_empty()
        })?;
        Some(Self {
            remote: remote.to_owned(),
            url: observed.url.clone(),
            scope: entry.install_scope.clone(),
            branch: entry.branch.clone(),
            arch: entry.deployed_arch.clone(),
        })
    }

    /// The configured remote that is still this source: same name, same installation, same URL.
    #[must_use]
    pub(crate) fn current_remote<'a>(&self, remotes: &'a [RemoteInfo]) -> Option<&'a RemoteInfo> {
        let url = normalise_repository_url(&self.url);
        remotes.iter().find(|remote| {
            remote.name == self.remote
                && InstallScope::from_remote_options(&remote.options) == self.scope
                && normalise_repository_url(&remote.url) == url
        })
    }
}

/// One remote in `flatpak remotes`.
#[derive(Clone, Debug, Default)]
pub(crate) struct RemoteInfo {
    pub name: String,
    pub title: String,
    pub url: String,
    pub options: String,
}

/// A summary of the permissions declared in an app's `[Context]` metadata.
/// Bazaar prints these as sortable "safety" chips; we render them as a bulleted
/// list in the permissions dialog and a compact pill row on the detail page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Permissions {
    pub shared: Vec<String>,
    pub sockets: Vec<String>,
    pub devices: Vec<String>,
    pub filesystems: Vec<String>,
    pub session_bus: Vec<String>,
    pub system_bus: Vec<String>,
    pub features: Vec<String>,
}

impl Permissions {
    pub(crate) fn is_empty(&self) -> bool {
        self.shared.is_empty()
            && self.sockets.is_empty()
            && self.devices.is_empty()
            && self.filesystems.is_empty()
            && self.session_bus.is_empty()
            && self.system_bus.is_empty()
            && self.features.is_empty()
    }

    /// The short "stat pills" the Bazaar detail page renders in a row across
    /// the app hero (Safe / Network / Filesystem / etc). We derive them here so
    /// the view stays UI-only.
    pub(crate) fn stat_pills(&self) -> Vec<(&'static str, bool)> {
        let network = self.shared.iter().any(|s| s == "network");
        let home = self.filesystems.iter().any(|f| f.starts_with("home"));
        let host_fs = self.filesystems.iter().any(|f| f.starts_with("host"));
        let x11 = self
            .sockets
            .iter()
            .any(|s| s == "x11" || s == "fallback-x11");
        let wayland = self.sockets.iter().any(|s| s == "wayland");
        let system_bus_open = !self.system_bus.is_empty();
        vec![
            ("Network", network),
            ("Wayland", wayland),
            ("X11", x11),
            ("Home", home),
            ("Host FS", host_fs),
            ("System Bus", system_bus_open),
        ]
    }
}
