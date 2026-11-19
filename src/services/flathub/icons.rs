/// Screenshot from the per-app appstream endpoint.
/// Each screenshot has `sizes` (list of {src, width, height}) and a `caption`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub(crate) struct FlathubScreenshot {
    #[serde(default)]
    pub sizes: Vec<FlathubScreenshotSize>,
    // Missing and null captions both mean no caption; neither invalidates the page.
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub caption: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[allow(dead_code, reason = "Retain the deserialized upstream API wire shape")] // Preserve both dimensions of the upstream wire shape.
pub(crate) struct FlathubScreenshotSize {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub src: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub width: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub height: String,
}

impl FlathubScreenshot {
    /// Bounded download candidates ordered by closeness to the 624 px stage.
    /// Duplicate URLs and empty records are removed before the retry loop.
    pub(crate) fn download_urls(&self) -> Vec<&str> {
        let mut sizes: Vec<&FlathubScreenshotSize> = self
            .sizes
            .iter()
            .filter(|size| !size.src.trim().is_empty())
            .collect();
        sizes.sort_by_key(|size| {
            let width = size.width.parse::<i32>().unwrap_or(i32::MAX);
            (width.saturating_sub(624).unsigned_abs(), width)
        });
        let mut urls = Vec::new();
        for size in sizes {
            if !urls.contains(&size.src.as_str()) {
                urls.push(size.src.as_str());
                if urls.len() == MAX_SCREENSHOT_DOWNLOAD_CANDIDATES {
                    break;
                }
            }
        }
        urls
    }

    /// Get a reasonable thumbnail URL (prefer smallest).
    pub(crate) fn thumb_url(&self) -> Option<&str> {
        self.sizes
            .iter()
            .filter(|s| !s.src.trim().is_empty())
            .min_by_key(|s| s.width.parse::<u32>().unwrap_or(9999))
            .map(|s| s.src.as_str())
    }

    /// The size whose width is nearest `width` px.
    pub(crate) fn url_near(&self, width: i32) -> Option<&str> {
        self.sizes
            .iter()
            .filter(|s| !s.src.trim().is_empty())
            .min_by_key(|s| {
                let w = s.width.parse::<i32>().unwrap_or(9999);
                (w - width).unsigned_abs()
            })
            .map(|s| s.src.as_str())
    }
}

/// Fetch app metadata from the Flathub v2 per-app API.
/// Used for detail page enrichment (screenshots, full description).
pub(crate) fn flathub_app_info(app_id: &str) -> Result<FlathubAppInfo, String> {
    flathub_app_info_with_fixture(app_id, fixture_enabled())
}

// Supply the fixture flag explicitly so concurrent tests do not mutate process-global state.
pub(crate) fn flathub_app_info_with_fixture(
    app_id: &str,
    fixture: bool,
) -> Result<FlathubAppInfo, String> {
    flathub_app_info_with_http(
        app_id,
        fixture,
        &CurlMetadataHttp,
        &OperationCancellation::default(),
    )
}

fn flathub_app_info_with_http(
    app_id: &str,
    fixture: bool,
    http: &dyn MetadataHttp,
    cancellation: &OperationCancellation,
) -> Result<FlathubAppInfo, String> {
    if fixture {
        return Ok(FlathubAppInfo {
            name: app_id.to_owned(),
            project_license: "GPL-3.0-or-later".into(),
            content_rating: fixture_detail_context().content_rating,
            releases: fixture_detail_context().releases,
            urls: fixture_detail_context().urls,
            screenshots: Vec::new(),
            ..Default::default()
        });
    }
    crate::flatpak_args::validate_app_id(app_id)?;
    let url = format!("https://flathub.org/api/v2/appstream/{app_id}");
    let raw = http
        .get(&url, MetadataRoute::Appstream, cancellation)
        .map_err(|error| error.to_string())?;
    parse_flathub_app_info(&raw)
}

/// Returns a bounded decoded icon from the retained cache or remote source.
pub(crate) fn cached_icon_at(
    icons: &crate::roots::SecureDir,
    app_id: &str,
    icon_url: &str,
    is_cancelled: &dyn Fn() -> bool,
) -> Option<crate::bounded_image::RemoteImage> {
    let key = crate::bounded_image::icon_cache_key(app_id, icon_url)?;
    match crate::bounded_image::load_cached_icon_at(icons, &key, is_cancelled) {
        Ok(Some(image)) => Some(image),
        Ok(None) | Err(_) => crate::bounded_image::fetch_icon_at(icons, &key, is_cancelled).ok(),
    }
}

/// Decode an app icon already present under the held icon-cache descriptor,
/// downloading nothing. A stale SVG, oversized file, or compact bomb is ignored
/// instead of becoming an accepted cache hit.
pub(crate) fn existing_icon_at(
    icons: &crate::roots::SecureDir,
    app_id: &str,
    icon_url: &str,
    is_cancelled: &dyn Fn() -> bool,
) -> Option<crate::bounded_image::RemoteImage> {
    let key = crate::bounded_image::icon_cache_key(app_id, icon_url)?;
    match crate::bounded_image::load_cached_icon_at(icons, &key, is_cancelled) {
        Ok(Some(image)) => Some(image),
        Ok(None) | Err(_) => None,
    }
}
