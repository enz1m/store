pub(crate) fn list_installed_cancellable(
    cancellation: OperationCancellation,
) -> Result<Vec<AppEntry>, String> {
    let executor = ProcessFlatpakExecutor::cancellable(cancellation.clone());
    let mut entries = list_installed_with_executor(fixture_enabled(), &executor)?;
    local_icons::LocalIcons::from_env().hydrate(&mut entries, &|| cancellation.is_cancelled());
    Ok(entries)
}

/// Installed-app read with the fixture flag supplied directly.
pub(crate) fn list_installed_with_fixture(fixture: bool) -> Result<Vec<AppEntry>, String> {
    list_installed_with_executor(fixture, &ProcessFlatpakExecutor::default())
}

// Requested columns and parser indices must change together; missing installation data stays Unknown.
const INSTALLED_COLUMNS_ARG: &str = "--columns=application,name,version,branch,origin,size,installation,description,arch:f,active:f";

fn list_installed_with_executor(
    fixture: bool,
    executor: &dyn FlatpakExecutor,
) -> Result<Vec<AppEntry>, String> {
    if fixture {
        return Ok(fixture_installed());
    }
    let raw = run_flatpak(executor, &["list", "--app", INSTALLED_COLUMNS_ARG])?;
    let mut apps = Vec::new();
    for cols in parse_columns(&raw) {
        if cols.len() < 5 {
            continue;
        }
        let id = cols[0].clone();
        if crate::flatpak_args::validate_app_id(&id).is_err() {
            continue;
        }
        let name = if cols[1].is_empty() {
            id.clone()
        } else {
            cols[1].clone()
        };
        apps.push(AppEntry {
            id,
            name,
            version: cols.get(2).cloned().unwrap_or_default(),
            branch: cols.get(3).cloned().unwrap_or_default(),
            origin: cols.get(4).cloned().unwrap_or_default(),
            size: cols.get(5).cloned().unwrap_or_default(),
            installed: true,
            deployed_arch: cols.get(8).cloned().unwrap_or_default(),
            deployed_commit: cols.get(9).cloned().unwrap_or_default(),
            summary: cols
                .get(7)
                .map(|value| {
                    value
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(512)
                        .collect()
                })
                .unwrap_or_default(),
            // Missing installation cells must not imply user scope.
            install_scope: InstallScope::from_installation_cell(
                cols.get(6).map(String::as_str).unwrap_or_default(),
            ),
            ..Default::default()
        });
    }
    Ok(apps)
}

#[derive(Clone, Debug)]
pub(crate) struct RepositoryInventory {
    pub remotes: Vec<RemoteInfo>,
    pub enzimos_apps: Option<Vec<AppEntry>>,
}

pub(crate) fn list_remotes_cancellable(
    cache: Option<&crate::roots::CacheCapability>,
    cancellation: OperationCancellation,
) -> Result<RepositoryInventory, String> {
    let executor = ProcessFlatpakExecutor::cancellable(cancellation.clone());
    let fixture = fixture_enabled();
    let remotes = list_remotes_with_executor(fixture, &executor)?;
    let enzimos_apps = list_enzimos_apps(&executor, &remotes, cache, &cancellation).ok();
    Ok(RepositoryInventory {
        remotes,
        enzimos_apps,
    })
}

fn list_enzimos_apps(
    executor: &dyn FlatpakExecutor,
    remotes: &[RemoteInfo],
    cache: Option<&crate::roots::CacheCapability>,
    cancellation: &OperationCancellation,
) -> Result<Vec<AppEntry>, String> {
    let is_cancelled = &|| cancellation.is_cancelled();
    let mut sources = Vec::new();
    for (scope, flag) in [
        (InstallScope::User, "--user"),
        (InstallScope::System, "--system"),
    ] {
        if let Some(remote) = remotes.iter().find(|remote| {
            InstallScope::from_remote_options(&remote.options) == scope
                && matches_catalog_origin(remote, ENZIMOS_REPOSITORY.origin)
        }) {
            sources.push((remote, flag));
        }
    }
    let mut failure = None;
    for (remote, flag) in &sources {
        let mut args = vec![
            "remote-ls",
            flag,
            "--app",
            "--columns=application,name,description,version,branch",
            "--",
            &remote.name,
        ];
        let raw = run_flatpak(executor, &args).or_else(|error| {
            args.insert(1, "--cached");
            run_flatpak(executor, &args).map_err(|_| error)
        });
        match raw {
            Ok(raw) => {
                let mut entries = parse_enzimos_apps(&raw);
                if !entries.is_empty()
                    && let Ok(arch) = run_flatpak(executor, &["--default-arch"])
                {
                    let icons = local_icons::LocalIcons::from_env();
                    for (source, _) in &sources {
                        icons.hydrate_appstream(&mut entries, source, arch.trim(), is_cancelled);
                    }
                }
                if let Some(cache) = cache {
                    hydrate_enzimos_icons(&mut entries, cache.icons(), is_cancelled);
                }
                if !entries.is_empty() && !is_cancelled() {
                    enrich_enzimos_listings(&mut entries, cancellation);
                }
                return Ok(entries);
            }
            Err(error) => failure = Some(error),
        }
    }
    failure.map_or_else(|| Ok(Vec::new()), Err)
}

#[derive(serde::Deserialize)]
struct EnzimosCatalogue {
    apps: Vec<EnzimosListing>,
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct EnzimosListing {
    id: String,
    title: String,
    summary: String,
    description: String,
    developer: String,
    version: String,
    license: String,
    homepage: String,
    support_url: String,
    release_notes: String,
    screenshots: Vec<EnzimosScreenshot>,
}

#[derive(serde::Deserialize)]
struct EnzimosScreenshot {
    url: String,
    #[serde(default)]
    caption: String,
}

fn enrich_enzimos_listings(entries: &mut [AppEntry], cancellation: &OperationCancellation) {
    const CATALOGUE: &str = "https://flatpak.enzim.dev/catalog.json";
    let Ok(raw) = CurlMetadataHttp.get(CATALOGUE, MetadataRoute::EnzimosCatalogue, cancellation)
    else {
        return;
    };
    apply_enzimos_listings(entries, &raw);
}

fn apply_enzimos_listings(entries: &mut [AppEntry], raw: &str) {
    let Ok(catalogue) = serde_json::from_str::<EnzimosCatalogue>(raw) else {
        return;
    };
    let mut listings: std::collections::HashMap<_, _> = catalogue
        .apps
        .into_iter()
        .take(512)
        .map(|listing| (listing.id.clone(), listing))
        .collect();
    for entry in entries {
        let Some(listing) = listings.remove(&entry.id) else {
            continue;
        };
        let text = |value: &str, limit| -> String {
            value
                .chars()
                .filter(|ch| !ch.is_control())
                .take(limit)
                .collect()
        };
        if !listing.title.is_empty() {
            entry.name = text(&listing.title, 128);
        }
        if !listing.summary.is_empty() {
            entry.summary = text(&listing.summary, 512);
        }
        entry.description = text(&listing.description, 4096);
        entry.publisher = text(&listing.developer, 128);
        entry.license = text(&listing.license, 128);
        let screenshot_candidates: Vec<_> = listing
            .screenshots
            .into_iter()
            .take(MAX_DETAIL_SCREENSHOTS)
            .filter_map(|shot| {
                let prefix = format!("/repo/media/{}/screenshots/", entry.id);
                let file = shot.url.strip_prefix(&prefix)?;
                if !file.rsplit_once('.').is_some_and(|(name, extension)| {
                    !name.is_empty() && matches!(extension, "png" | "jpg" | "jpeg" | "webp")
                }) || file.len() > 128
                    || !file
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                {
                    return None;
                }
                Some(FlathubScreenshot {
                    sizes: vec![FlathubScreenshotSize {
                        src: format!(
                            "{}media/{}/screenshots/{file}",
                            ENZIMOS_REPOSITORY.url, entry.id
                        ),
                        ..Default::default()
                    }],
                    caption: text(&shot.caption, 512),
                })
            })
            .collect();
        let releases = if listing.version.is_empty() || listing.release_notes.is_empty() {
            Vec::new()
        } else {
            vec![FlathubRelease {
                version: text(&listing.version, 128),
                description: text(&listing.release_notes, 4096),
                ..Default::default()
            }]
        };
        let link = |url: String| -> String {
            if url.len() <= 2048
                && url.starts_with("https://")
                && !url.chars().any(char::is_control)
            {
                url
            } else {
                String::new()
            }
        };
        entry.enzimos_detail = Some(std::sync::Arc::new(DetailContext {
            metadata_state: DetailRouteState::Known,
            license: entry.license.clone(),
            license_kind: classify_license(&entry.license),
            releases,
            screenshot_count: screenshot_candidates.len(),
            screenshot_candidates,
            urls: FlathubUrls {
                homepage: link(listing.homepage),
                help: link(listing.support_url),
                ..Default::default()
            },
            ..Default::default()
        }));
    }
}

fn hydrate_enzimos_icons(
    entries: &mut [AppEntry],
    icons: &crate::roots::SecureDir,
    is_cancelled: &dyn Fn() -> bool,
) {
    // Offline installation media may leave the remote's AppStream cache empty.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let cancelled = || is_cancelled() || std::time::Instant::now() >= deadline;
    for entry in entries
        .iter_mut()
        .filter(|entry| entry.icon_image.is_none())
    {
        if cancelled() {
            break;
        }
        let url = format!("{}media/{}/icon.png", ENZIMOS_REPOSITORY.url, entry.id);
        entry.icon_image = cached_icon_at(icons, &entry.id, &url, &cancelled)
            .map(crate::bounded_image::RenderImage::Remote);
    }
}

#[cfg(test)]
#[test]
fn listing_enrichment_keeps_flatpak_refs_and_only_loads_their_own_media() {
    let mut entries = parse_enzimos_apps("org.example.App\t\t\t1\tstable\n");
    apply_enzimos_listings(
        &mut entries,
        r#"{
        "apps": [
            {"id":"org.example.Unlisted","title":"Unlisted"},
            {"id":"org.example.App","title":"Example","summary":"An example",
             "screenshots":[
                {"url":"/repo/media/org.example.Unlisted/screenshots/wrong.png"},
                {"url":"/repo/media/org.example.App/screenshots/../icon.png"},
                {"url":"/repo/media/org.example.App/screenshots/right.png"}
             ]}
        ]
    }"#,
    );
    let [entry] = entries.as_slice() else {
        panic!("unexpected catalogue refs")
    };
    assert_eq!(entry.id, "org.example.App");
    assert_eq!(entry.name, "Example");
    assert_eq!(entry.summary, "An example");
    let detail = entry.enzimos_detail.as_ref().expect("listing details");
    assert_eq!(detail.screenshot_count, 1);
    assert_eq!(
        detail.screenshot_candidates[0].sizes[0].src,
        "https://flatpak.enzim.dev/repo/media/org.example.App/screenshots/right.png"
    );
}

fn parse_enzimos_apps(raw: &str) -> Vec<AppEntry> {
    let mut seen = std::collections::HashSet::new();
    parse_columns(raw)
        .into_iter()
        .filter_map(|cells| {
            let [id, name, summary, version, branch] = cells.as_slice() else {
                return None;
            };
            if branch != "stable"
                || crate::flatpak_args::validate_app_id(id).is_err()
                || !seen.insert(id.clone())
            {
                return None;
            }
            let display = |text: &str| -> String {
                text.chars()
                    .filter(|ch| !ch.is_control())
                    .take(512)
                    .collect()
            };
            Some(AppEntry {
                id: id.clone(),
                name: if name.is_empty() {
                    id.clone()
                } else {
                    display(name)
                },
                summary: display(summary),
                version: display(version),
                branch: branch.clone(),
                origin: ENZIMOS_REPOSITORY.origin.into(),
                ..Default::default()
            })
        })
        .take(128)
        .collect()
}

/// Remote-list read with the fixture flag supplied directly.
pub(crate) fn list_remotes_with_fixture(fixture: bool) -> Result<Vec<RemoteInfo>, String> {
    list_remotes_with_executor(fixture, &ProcessFlatpakExecutor::default())
}

fn list_remotes_with_executor(
    fixture: bool,
    executor: &dyn FlatpakExecutor,
) -> Result<Vec<RemoteInfo>, String> {
    if fixture {
        return Ok(fixture_remotes());
    }
    let raw = run_flatpak(executor, &["remotes", "--columns=name,title,url,options"])?;
    let mut remotes = Vec::new();
    for cols in parse_columns(&raw) {
        if cols.is_empty() {
            continue;
        }
        let name = cols.first().cloned().unwrap_or_default();
        if crate::flatpak_args::validate_remote_name(&name).is_err() {
            continue;
        }
        remotes.push(RemoteInfo {
            name,
            title: cols.get(1).cloned().unwrap_or_default(),
            url: cols.get(2).cloned().unwrap_or_default(),
            options: cols.get(3).cloned().unwrap_or_default(),
        });
    }
    Ok(remotes)
}

/// An explicitly queried installation and its authority to refresh appstream metadata.
#[derive(Clone, Copy, Debug)]
struct UpdateReadScope {
    /// The `flatpak` option selecting the installation.
    option: &'static str,
    /// Whether `flatpak update --appstream <option>` is an unprivileged write
    /// this build may perform first.
    may_refresh_appstream: bool,
}

/// Query both installations unconditionally; an empty scope result does not describe another scope.
impl UpdateReadScope {
    // The installation this entry selects, as the value an
    // [`AvailableUpdate`] records.
    //
    // Derived from `option` rather than stored beside it so the two cannot
    // disagree: the flag that was sent to flatpak *is* the scope that
    // answered.
    fn installation(self) -> InstallScope {
        match self.option {
            "--user" => InstallScope::User,
            "--system" => InstallScope::System,
            named => InstallScope::Named(named.trim_start_matches("--installation=").to_owned()),
        }
    }
}

const UPDATE_READ_SCOPES: [UpdateReadScope; 2] = [
    UpdateReadScope {
        option: "--user",
        may_refresh_appstream: true,
    },
    UpdateReadScope {
        // `flatpak update --appstream --system` writes `/var/lib/flatpak`
        // through the privileged system helper, so this build must not
        // attempt it silently at boot. The system scope is read against the
        // appstream data that is already on disk; `remote-ls` itself is an
        // unprivileged read.
        option: "--system",
        may_refresh_appstream: false,
    },
];
