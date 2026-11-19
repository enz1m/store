include!("flathub/app_info.rs");
include!("flathub/verification.rs");
include!("flathub/ratings.rs");
include!("flathub/detail_types.rs");
include!("flathub/license.rs");
include!("flathub/oars.rs");
include!("flathub/status_fetch.rs");
include!("flathub/detail_fetch.rs");
include!("flathub/icons.rs");
include!("flathub/http_errors.rs");

// The publisher-link route lives in `crate::url_launch::UrlLauncher`,
// driven by the framework's XDG Desktop Portal OpenURI transport
// (`enzui_core::portal::open_uri`). There is no subprocess between
// Store and the desktop handler, and the scheme allow-list is enforced
// inside `UrlLauncher::start` on the URI parsed once by
// `OpenUri::parse`.

struct FrameworkMetadataTransport;

impl MetadataTransport for FrameworkMetadataTransport {
    fn fetch(
        &self,
        request: &RemoteRequest,
        policy: &UrlPolicy,
        limits: &TransportLimits,
        probe: &CancelProbe<'_>,
        progress: &mut dyn FnMut(TransferProgress),
    ) -> Result<TransportResponse, RemoteTransportError> {
        CurlTransport::new().fetch_with_cancel_probe(request, policy, limits, probe, progress)
    }
}

fn metadata_url_policy() -> UrlPolicy {
    // Fixed metadata origins never delegate authority to a redirect target.
    UrlPolicy {
        allow_http: false,
        allow_private_network: false,
        max_redirects: 0,
    }
}

fn checked_metadata_request(
    url: &str,
    route: MetadataRoute,
    policy: &UrlPolicy,
) -> Result<RemoteRequest, HttpFetchError> {
    let request =
        RemoteRequest::checked(url, policy).map_err(|verdict| HttpFetchError::PolicyBlocked {
            code: verdict.reason(),
        })?;
    let safe = request.safe_url();
    if safe.scheme() != Scheme::Https
        || safe.host() != route.origin_host()
        || safe.port().is_some_and(|port| port != 443)
    {
        return Err(HttpFetchError::PolicyBlocked {
            code: "origin-refused",
        });
    }
    Ok(request)
}

fn header_limit_code(kind: HeaderLimitKind) -> &'static str {
    match kind {
        HeaderLimitKind::Count { .. } => "header-count",
        HeaderLimitKind::Bytes { .. } => "header-bytes",
        HeaderLimitKind::DuplicateSingleton { .. } => "duplicate-singleton",
        HeaderLimitKind::ConflictingFraming => "conflicting-framing",
    }
}

fn address_class_code(class: AddressClass) -> &'static str {
    match class {
        AddressClass::Public => "public-peer-mismatch",
        AddressClass::Loopback => "loopback",
        AddressClass::Private => "private",
        AddressClass::LinkLocal => "link-local",
        AddressClass::Special => "special-use",
    }
}

fn is_tls_backend_code(code: u32) -> bool {
    // Stable libcurl CURLcode values for TLS setup, certificate, cipher,
    // engine, revocation, pinning, and shutdown failures.
    const TLS_CODES: &[u32] = &[
        35, 51, 53, 54, 58, 59, 60, 64, 66, 77, 80, 82, 83, 90, 91, 98,
    ];
    TLS_CODES.contains(&code)
}

fn map_metadata_transport_error(error: RemoteTransportError) -> HttpFetchError {
    match error {
        RemoteTransportError::Url(UrlVerdict::TooManyRedirects { .. }) => {
            HttpFetchError::PolicyBlocked {
                code: "redirect-refused",
            }
        }
        RemoteTransportError::Url(verdict) => HttpFetchError::PolicyBlocked {
            code: verdict.reason(),
        },
        RemoteTransportError::AddressBlocked(class) => HttpFetchError::PolicyBlocked {
            code: address_class_code(class),
        },
        RemoteTransportError::Dns => HttpFetchError::Transport {
            code: "dns",
            backend_code: None,
        },
        RemoteTransportError::Cancelled(_) => HttpFetchError::Cancelled,
        RemoteTransportError::ByteLimit { received, limit } => HttpFetchError::BodyTooLarge {
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
            received: usize::try_from(received).unwrap_or(usize::MAX),
        },
        RemoteTransportError::Timeout => HttpFetchError::DeadlineExceeded {
            milliseconds: HTTP_TOTAL_DEADLINE_MS,
        },
        RemoteTransportError::Redirect => HttpFetchError::Transport {
            code: "redirect",
            backend_code: None,
        },
        RemoteTransportError::HttpStatus(status) => HttpFetchError::HttpStatus { status },
        RemoteTransportError::ContentEncoding => HttpFetchError::Transport {
            code: "content-encoding",
            backend_code: None,
        },
        RemoteTransportError::HeaderLimit(kind) => HttpFetchError::HeaderTooLarge {
            code: header_limit_code(kind),
        },
        RemoteTransportError::Backend(code) if is_tls_backend_code(code) => {
            HttpFetchError::Tls { code }
        }
        RemoteTransportError::Backend(code) => HttpFetchError::Transport {
            code: "backend",
            backend_code: Some(code),
        },
        _ => HttpFetchError::Transport {
            code: "unknown",
            backend_code: None,
        },
    }
}

fn run_metadata_transport(
    url: &str,
    route: MetadataRoute,
    cancellation: &dyn Fn() -> bool,
    transport: &dyn MetadataTransport,
) -> Result<String, HttpFetchError> {
    if cancellation() {
        return Err(HttpFetchError::Cancelled);
    }
    let policy = metadata_url_policy();
    let limits = route.transport_limits();
    let request = checked_metadata_request(url, route, &policy)?;
    let signal = CancelSignal::new();
    let probe = CancelProbe::new(&signal, cancellation, CancelCause::Cancelled);
    let mut progress = |_progress: TransferProgress| {};
    let response = transport
        .fetch(&request, &policy, &limits, &probe, &mut progress)
        .map_err(map_metadata_transport_error)?;
    if response.body.len() > route.body_limit() {
        return Err(HttpFetchError::BodyTooLarge {
            limit: route.body_limit(),
            received: response.body.len(),
        });
    }
    String::from_utf8(response.body).map_err(|_| HttpFetchError::InvalidUtf8)
}

fn run_metadata_cancellable(
    url: &str,
    route: MetadataRoute,
    cancellation: &OperationCancellation,
) -> Result<String, HttpFetchError> {
    run_metadata_transport(
        url,
        route,
        &|| cancellation.is_cancelled(),
        &FrameworkMetadataTransport,
    )
}

/// The narrow metadata-transport seam: every detail-page HTTP step goes
/// through this, so an owner can inject a transport and every step observes
/// the same cancellation flag.
trait MetadataHttp: Send + Sync {
    fn get(
        &self,
        url: &str,
        route: MetadataRoute,
        cancellation: &OperationCancellation,
    ) -> Result<String, HttpFetchError>;
}

/// The production metadata transport over the framework's policy-safe libcurl
/// owner.
struct CurlMetadataHttp;

impl MetadataHttp for CurlMetadataHttp {
    fn get(
        &self,
        url: &str,
        route: MetadataRoute,
        cancellation: &OperationCancellation,
    ) -> Result<String, HttpFetchError> {
        run_metadata_cancellable(url, route, cancellation)
    }
}

const MAX_METADATA_STRING_BYTES: usize = 1024 * 1024;
const MAX_METADATA_URL_BYTES: usize = 8 * 1024;
const MAX_SCREENSHOT_RECORDS: usize = 64;
const MAX_SCREENSHOT_SIZES: usize = 16;
const MAX_RELEASE_RECORDS: usize = 256;
const MAX_OARS_RECORDS: usize = 64;
const MAX_RATINGS_RECORDS: usize = 100_000;
const MAX_RATING_COUNT: u64 = 1_000_000_000;
const MAX_RATINGS_AGGREGATE: u64 = 10_000_000_000_000;

fn metadata_string_is_bounded(value: &str) -> bool {
    value.len() <= MAX_METADATA_STRING_BYTES
}

fn app_info_is_bounded(info: &FlathubAppInfo) -> bool {
    let scalar_strings = [
        &info.name,
        &info.summary,
        &info.description,
        &info.developer_name,
        &info.project_license,
        &info.icon,
        &info.end_of_life,
        &info.is_eol_message,
    ];
    if scalar_strings
        .iter()
        .any(|value| !metadata_string_is_bounded(value))
        || info.screenshots.len() > MAX_SCREENSHOT_RECORDS
        || info.releases.len() > MAX_RELEASE_RECORDS
        || info.content_rating.len() > MAX_OARS_RECORDS
    {
        return false;
    }
    let urls = [
        &info.urls.homepage,
        &info.urls.bugtracker,
        &info.urls.donation,
        &info.urls.help,
        &info.urls.vcs_browser,
        &info.urls.contribute,
        &info.urls.translate,
        &info.urls.faq,
        &info.urls.contact,
    ];
    if urls
        .iter()
        .any(|value| value.len() > MAX_METADATA_URL_BYTES)
    {
        return false;
    }
    if info.screenshots.iter().any(|screenshot| {
        screenshot.caption.len() > MAX_METADATA_STRING_BYTES
            || screenshot.sizes.len() > MAX_SCREENSHOT_SIZES
            || screenshot.sizes.iter().any(|size| {
                size.src.len() > MAX_METADATA_URL_BYTES
                    || size.width.len() > 16
                    || size.height.len() > 16
            })
    }) {
        return false;
    }
    let mut release_bytes = 0usize;
    for release in &info.releases {
        let Some(next) = release_bytes
            .checked_add(release.version.len())
            .and_then(|n| n.checked_add(release.timestamp.len()))
            .and_then(|n| n.checked_add(release.description.len()))
            .and_then(|n| n.checked_add(release.release_type.len()))
        else {
            return false;
        };
        release_bytes = next;
        if release_bytes > MAX_METADATA_STRING_BYTES {
            return false;
        }
    }
    info.content_rating
        .iter()
        .all(|(key, value)| key.len() <= 128 && value.len() <= 128)
}

// Parse the Flathub appstream v2 JSON into our struct.
fn parse_flathub_app_info(raw: &str) -> Result<FlathubAppInfo, String> {
    let document: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("JSON parse: {e}"))?;
    let Some(object) = document.as_object() else {
        return Err("appstream response is not an object".to_owned());
    };
    if ![
        "name",
        "summary",
        "description",
        "project_license",
        "screenshots",
        "content_rating",
        "releases",
        "urls",
    ]
    .into_iter()
    .any(|field| object.contains_key(field))
    {
        return Err("appstream response has no appstream fields".to_owned());
    }
    let info: FlathubAppInfo =
        serde_json::from_value(document).map_err(|e| format!("JSON parse: {e}"))?;
    if !app_info_is_bounded(&info) {
        return Err("appstream object exceeds its decoded limits".to_owned());
    }
    Ok(info)
}

/// Parse the Flathub verification-status endpoint JSON.
pub(crate) fn parse_verification_status(raw: &str) -> Result<FlathubVerificationStatus, String> {
    let status: FlathubVerificationStatus =
        serde_json::from_str(raw).map_err(|e| format!("JSON parse: {e}"))?;
    if status.method.len() > 128 {
        return Err("verification object exceeds its decoded limits".to_owned());
    }
    Ok(status)
}

/// Parse the Flathub stats endpoint JSON.
pub(crate) fn parse_app_stats(raw: &str) -> Result<FlathubAppStats, String> {
    let stats: FlathubAppStats =
        serde_json::from_str(raw).map_err(|e| format!("JSON parse: {e}"))?;
    if [
        stats.installs_total,
        stats.installs_last_month,
        stats.installs_last_7_days,
    ]
    .into_iter()
    .any(|count| count > MAX_RATINGS_AGGREGATE)
    {
        return Err("stats object exceeds its decoded limits".to_owned());
    }
    Ok(stats)
}

/// Parse collection hits and pagination so ingestion is bounded by pages rather than app count.
pub(crate) fn parse_collection_page(
    raw: &str,
) -> Result<FlathubCollectionResponse, CatalogParseError> {
    serde_json::from_str(raw).map_err(|e| CatalogParseError::from_serde(&e))
}

// Catalogue-owner form whose cancellation predicate is polled by libcurl.
pub(crate) fn http_get_cancellable(
    url: &str,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<String, HttpFetchError> {
    run_metadata_transport(
        url,
        MetadataRoute::Catalogue,
        is_cancelled,
        &FrameworkMetadataTransport,
    )
}

/// Convert a Flathub collection hit into an `AppEntry`.
///
/// Invalid external IDs are dropped here, before they can enter the catalogue
/// or be copied into an effect-producing message.
pub(crate) fn hit_to_entry(hit: &FlathubCollectionHit) -> Option<AppEntry> {
    let app_id = if !hit.app_id.is_empty() {
        &hit.app_id
    } else {
        &hit.id
    };
    crate::flatpak_args::validate_app_id(app_id).ok()?;
    Some(AppEntry {
        id: app_id.clone(),
        name: if hit.name.is_empty() {
            app_id.clone()
        } else {
            hit.name.clone()
        },
        summary: hit.summary.clone(),
        // Normalize HTML paragraph breaks and interior indentation before any view renders the description.
        description: strip_html(&hit.description),
        publisher: hit.developer_name.clone(),
        origin: "flathub".to_owned(),
        verified: hit.verification_verified,
        downloads: hit.installs_last_month,
        icon_url: hit.icon.clone(),
        // Normalise main-category to lowercase so filters can match on
        // a stable key regardless of API casing quirks. Empty when
        // Flathub didn't tag the app.
        category: hit.main_categories.to_ascii_lowercase(),
        ..Default::default()
    })
}

/// Discover ranking rails derived from the complete catalogue, with every rail entry retained in the index.
#[derive(Clone, Debug, Default)]
pub(crate) struct FlathubCatalogs {
    pub popular: Vec<AppEntry>,
    pub trending: Vec<AppEntry>,
    pub new: Vec<AppEntry>,
    pub updated: Vec<AppEntry>,
}

impl PartialEq for FlathubCatalogs {
    fn eq(&self, other: &Self) -> bool {
        self.popular == other.popular
            && self.trending == other.trending
            && self.new == other.new
            && self.updated == other.updated
    }
}

/// Download and cache icons for a list of app entries.
/// Returns a map of app_id → validated cached RGBA image.
/// This must run off the UI thread.
/// A screenshot ready for the detail strip. Remote screenshots carry an
/// RGBA-only source; the trusted-bundled variant is limited to app fixtures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Screenshot {
    pub caption: String,
    pub image: crate::bounded_image::RenderImage,
}

/// Terminal result of one app's screenshot request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ScreenshotFetchResult {
    /// The app advertises no screenshots.
    KnownEmpty,
    /// Every selected screenshot produced a bounded image.
    Complete(Vec<Screenshot>),
    /// At least one selected screenshot loaded and at least one did not.
    Partial(Vec<Screenshot>),
    /// A bounded disk image is shown because its network replacement failed.
    CachedStale(Vec<Screenshot>),
    /// Metadata failed, or no advertised screenshot could be decoded.
    Failed,
    /// The owning generation was cancelled before the request settled. Callers
    /// must treat this as inert, never as an empty or failed gallery.
    Cancelled,
}

/// At most three differently-sized URLs are attempted for one screenshot.
/// Combined with the four-screenshot cap this limits one detail request to
/// twelve image transports even when every preferred rendition fails.
pub(crate) const MAX_SCREENSHOT_DOWNLOAD_CANDIDATES: usize = 3;
pub(crate) const MAX_DETAIL_SCREENSHOTS: usize = 4;

pub(crate) fn cache_screenshot_records_at(
    cache: &crate::roots::CacheCapability,
    app_id: &str,
    generation: u64,
    records: &[FlathubScreenshot],
    is_cancelled: &dyn Fn() -> bool,
) -> ScreenshotFetchResult {
    if records.is_empty() {
        return ScreenshotFetchResult::KnownEmpty;
    }

    let mut shots = Vec::new();
    let mut missing = false;
    let mut cached_stale = false;
    for ss in records.iter().take(MAX_DETAIL_SCREENSHOTS) {
        if is_cancelled() {
            return ScreenshotFetchResult::Cancelled;
        }
        let urls = ss.download_urls();
        if urls.is_empty() {
            missing = true;
            continue;
        }
        match cache_screenshot_candidates_at(
            cache,
            app_id,
            generation,
            &urls,
            &ss.caption,
            is_cancelled,
        ) {
            ScreenshotOutcome::Found(shot) => shots.push(shot),
            ScreenshotOutcome::CachedStale(shot) => {
                cached_stale = true;
                shots.push(shot);
            }
            ScreenshotOutcome::Missing => missing = true,
            ScreenshotOutcome::Cancelled => return ScreenshotFetchResult::Cancelled,
        }
    }
    if shots.is_empty() {
        ScreenshotFetchResult::Failed
    } else if cached_stale {
        ScreenshotFetchResult::CachedStale(shots)
    } else if missing {
        ScreenshotFetchResult::Partial(shots)
    } else {
        ScreenshotFetchResult::Complete(shots)
    }
}

// Cancellation is distinct from a miss so the candidate loop stops immediately.
enum ScreenshotOutcome {
    Found(Screenshot),
    CachedStale(Screenshot),
    Missing,
    Cancelled,
}

// A slot is a hit only when its embedded app-and-URL key matches this request.
fn cache_screenshot_candidates_at(
    cache: &crate::roots::CacheCapability,
    app_id: &str,
    generation: u64,
    urls: &[&str],
    caption: &str,
    is_cancelled: &dyn Fn() -> bool,
) -> ScreenshotOutcome {
    for url in urls
        .iter()
        .copied()
        .take(MAX_SCREENSHOT_DOWNLOAD_CANDIDATES)
    {
        if is_cancelled() {
            return ScreenshotOutcome::Cancelled;
        }
        let Some(key) = crate::bounded_image::screenshot_cache_key(app_id, generation, url) else {
            continue;
        };
        let cached = match crate::bounded_image::load_cached_raster_at(
            cache.screenshots(),
            &key,
            is_cancelled,
        ) {
            Ok(image) => image,
            Err(crate::bounded_image::ImageRefusal::Cancelled) => {
                return ScreenshotOutcome::Cancelled;
            }
            Err(_) => None,
        };
        match crate::bounded_image::fetch_screenshot_at(cache, &key, is_cancelled) {
            Ok(image) => {
                return ScreenshotOutcome::Found(Screenshot {
                    caption: caption.to_owned(),
                    image: crate::bounded_image::RenderImage::Remote(image),
                });
            }
            Err(crate::bounded_image::ImageRefusal::Cancelled) => {
                return ScreenshotOutcome::Cancelled;
            }
            Err(_) => {
                if let Some(image) = cached {
                    return ScreenshotOutcome::CachedStale(Screenshot {
                        caption: caption.to_owned(),
                        image: crate::bounded_image::RenderImage::Remote(image),
                    });
                }
            }
        }
    }
    ScreenshotOutcome::Missing
}

pub(crate) fn cache_icons_for(
    icons: &crate::roots::SecureDir,
    entries: &[AppEntry],
    is_cancelled: &dyn Fn() -> bool,
) -> std::collections::HashMap<String, crate::bounded_image::RemoteImage> {
    let mut result = std::collections::HashMap::new();
    for entry in entries {
        // A dropped or superseded catalog task aborts the icon loop between
        // entries, so no further icon is fetched or published.
        if is_cancelled() {
            break;
        }
        if entry.id.is_empty() || entry.icon_url.is_empty() {
            continue;
        }
        if let Some(icon) = cached_icon_at(icons, &entry.id, &entry.icon_url, is_cancelled) {
            result.insert(entry.id.clone(), icon);
        }
    }
    result
}
