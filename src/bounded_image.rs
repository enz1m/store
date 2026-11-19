//! Bounded transport, decode, cache, and renderer routing for raster images.
//!
//! Remote bytes never become a path-backed iced handle. Every cold fetch flows
//! through the shared policy-safe remote transport
//! ([`enzui_resources::curl_transport::CurlTransport`] under
//! [`UrlPolicy::DEFAULT`]): only screened HTTPS is admitted, credentials and
//! private/link-local/non-global/file destinations are refused before any
//! connection, and every DNS answer, redirect target, and connected peer is
//! re-screened under one total deadline. The body is capped at the exact image
//! ceiling while streaming, decoded by the framework under `DecodeLimits`, and
//! represented by [`RemoteImage`], a type which contains only validated RGBA.
//! Cache slots contain a versioned app-and-URL key beside the encoded body.
//! Publication uses `renameat` beneath a retained directory descriptor after
//! decode and generation checks. Remote cache hits become RGBA handles; only
//! [`TrustedBundledPath`] can create a path-backed renderer source.

use enzui_core::cancellation::CancelCause;
use enzui_core::decode_limits::DecodeLimits;
use enzui_core::url_policy::{TransportLimits, UrlPolicy};
use enzui_resources::curl_transport::{CancelProbe, CurlTransport};
use enzui_resources::{
    CacheIdentityDomain, CancelSignal, DecodeError, DecodedAsset, RemoteRequest, TransferProgress,
    TransportError as RemoteTransportError, cache_identity, decode_static, decode_static_resized,
};
use iced::widget::image::Handle;
#[cfg(test)]
use std::io::Read;
use std::path::Path;

pub(crate) const MAX_DOWNLOAD_BYTES: u64 = 12 * 1024 * 1024;
pub(crate) const MAX_ICON_DOWNLOAD_BYTES: u64 = 1024 * 1024;
// Several times the ~160 visible icons, so hashed slots rarely collide (128
// slots kept only ~90 of them and re-downloaded the rest on every launch).
pub(crate) const ICON_CACHE_SLOTS: u64 = 512;
pub(crate) const SCREENSHOT_CACHE_SLOTS: u64 = 24;
pub(crate) const IMAGE_CACHE_SCHEMA: u32 = 2;
pub(crate) const IMAGE_DECODER_VERSION: u32 = 1;
pub(crate) const MAX_IMAGE_WIDTH: u32 = 4_096;
pub(crate) const MAX_IMAGE_HEIGHT: u32 = 4_096;
pub(crate) const MAX_IMAGE_PIXELS: u64 = 16_777_216;
pub(crate) const MAX_IMAGE_RGBA_BYTES: u64 = 67_108_864;
const MAX_CACHE_KEY_BYTES: usize = 16 * 1024;
const CACHE_ENTRY_MAGIC: &[u8] = b"enzui-store-image-v2\n";

#[must_use]
pub(crate) fn production_limits() -> DecodeLimits {
    DecodeLimits {
        max_encoded_bytes: MAX_DOWNLOAD_BYTES,
        max_width: MAX_IMAGE_WIDTH,
        max_height: MAX_IMAGE_HEIGHT,
        max_pixels: MAX_IMAGE_PIXELS,
        max_rgba_bytes: MAX_IMAGE_RGBA_BYTES,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImageRefusal {
    /// The body streamed one byte past the exact image ceiling.
    TooLarge { cap: u64 },
    /// The URL, a resolved address, a redirect target, or the connected peer
    /// was refused by [`UrlPolicy`]: a non-HTTPS scheme, embedded credentials, a
    /// missing host, a `file://` destination, a private/link-local/non-global
    /// address, or too many redirects. No forbidden destination is connected to.
    /// `reason` is a stable, payload-free tag.
    Blocked { reason: &'static str },
    /// The owning consumer (a superseded detail generation) went away before the
    /// body completed, so nothing is decoded or published.
    Cancelled,
    /// A finite connect/read/total transport deadline expired.
    Timeout,
    /// Any other transport failure (name resolution, connect/read, HTTP status,
    /// content encoding, header flood, or backend fault).
    Transport(String),
    /// The complete body failed bounded raster decode.
    Decode(DecodeError),
}

impl ImageRefusal {
    #[cfg(test)]
    pub(crate) fn limit_reason(&self) -> Option<&'static str> {
        match self {
            Self::Decode(DecodeError::Limit(refused)) => Some((*refused).reason()),
            _ => None,
        }
    }
}

/// Renderer-safe decoded image with no path constructor or accessor. Remote
/// cache entries and local deployment icons share the RGBA-only representation
/// all the way into the widget tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteImage(Handle);

impl RemoteImage {
    fn from_decoded(asset: DecodedAsset) -> Self {
        Self(asset.handle)
    }

    pub(crate) fn handle(&self) -> Handle {
        self.0.clone()
    }
}

/// Explicit escape hatch for app-bundled visual fixtures/assets only.
///
/// A caller cannot put a runtime cache path in this type: the path must be a
/// static manifest-relative value, and absolute/parent-traversing paths are
/// rejected. Remote cache APIs return [`RemoteImage`] instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrustedBundledPath(std::path::PathBuf);

impl TrustedBundledPath {
    pub(crate) fn new(relative: &'static str) -> Self {
        let relative = Path::new(relative);
        assert!(
            !relative.is_absolute(),
            "bundled image path must be relative"
        );
        assert!(
            relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
            "bundled image path must not escape the app"
        );
        Self(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
    }

    pub(crate) fn handle(&self) -> Handle {
        Handle::from_path(&self.0)
    }
}

/// The renderer source used by screenshot/gallery production composition.
/// Only the visibly trusted variant can perform path decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RenderImage {
    Remote(RemoteImage),
    TrustedBundled(TrustedBundledPath),
}

impl RenderImage {
    pub(crate) fn icon_handle(&self) -> Handle {
        match self {
            Self::Remote(image) => {
                #[cfg(test)]
                ROUTES.with(|routes| routes.set((routes.get().0 + 1, routes.get().1)));
                image.handle()
            }
            Self::TrustedBundled(path) => path.handle(),
        }
    }

    pub(crate) fn gallery_handle(&self) -> Handle {
        match self {
            Self::Remote(image) => {
                #[cfg(test)]
                ROUTES.with(|routes| routes.set((routes.get().0, routes.get().1 + 1)));
                image.handle()
            }
            Self::TrustedBundled(path) => path.handle(),
        }
    }
}

#[cfg(test)]
thread_local! {
    static ROUTES: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(test)]
pub(crate) fn read_capped(reader: impl Read, cap: u64) -> Result<Vec<u8>, ImageRefusal> {
    let mut buf = Vec::new();
    reader
        .take(cap.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|error| ImageRefusal::Transport(error.to_string()))?;
    if buf.len() as u64 > cap {
        return Err(ImageRefusal::TooLarge { cap });
    }
    Ok(buf)
}

// A remote-image decode profile: a full raster or a downsampled icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecodeKind {
    Raster,
    Icon,
}

impl DecodeKind {
    fn label(self) -> &'static str {
        match self {
            Self::Raster => "screenshot",
            Self::Icon => "icon",
        }
    }

    fn max_encoded_bytes(self) -> u64 {
        match self {
            Self::Raster => MAX_DOWNLOAD_BYTES,
            Self::Icon => MAX_ICON_DOWNLOAD_BYTES,
        }
    }

    fn cache_slots(self) -> u64 {
        match self {
            Self::Raster => SCREENSHOT_CACHE_SLOTS,
            Self::Icon => ICON_CACHE_SLOTS,
        }
    }

    fn identity_domain(self) -> CacheIdentityDomain {
        match self {
            Self::Raster => CacheIdentityDomain::ScreenshotUrl,
            Self::Icon => CacheIdentityDomain::IconUrl,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageCacheKey {
    name: String,
    cache_content_key: String,
    normalized_url: String,
    detail_generation: Option<u64>,
    kind: DecodeKind,
}

impl ImageCacheKey {
    fn new(
        app_id: &str,
        url: &str,
        detail_generation: Option<u64>,
        kind: DecodeKind,
    ) -> Option<Self> {
        crate::flatpak_args::validate_app_id(app_id).ok()?;
        let normalized_url = url.trim();
        if normalized_url.is_empty() || normalized_url.len() > 8 * 1024 {
            return None;
        }
        let cache_content_key = format!(
            "schema={IMAGE_CACHE_SCHEMA}\ndecoder={IMAGE_DECODER_VERSION}\nkind={}\napp={app_id}\nurl={normalized_url}",
            kind.label()
        );
        if cache_content_key.len() > MAX_CACHE_KEY_BYTES {
            return None;
        }
        let digest = cache_identity(kind.identity_domain(), cache_content_key.as_bytes());
        let bucket = u64::from_str_radix(digest.get(..16)?, 16).ok()? % kind.cache_slots();
        Some(Self {
            name: format!("{}-v{IMAGE_CACHE_SCHEMA}-{bucket:03}.cache", kind.label()),
            cache_content_key,
            normalized_url: normalized_url.to_owned(),
            detail_generation,
            kind,
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    fn max_encoded_bytes(&self) -> u64 {
        self.kind.max_encoded_bytes()
    }
}

pub(crate) fn icon_cache_key(app_id: &str, url: &str) -> Option<ImageCacheKey> {
    ImageCacheKey::new(app_id, url, None, DecodeKind::Icon)
}

pub(crate) fn screenshot_cache_key(
    app_id: &str,
    generation: u64,
    url: &str,
) -> Option<ImageCacheKey> {
    if generation == 0 {
        return None;
    }
    ImageCacheKey::new(app_id, url, Some(generation), DecodeKind::Raster)
}

fn encode_cache_entry(key: &ImageCacheKey, bytes: &[u8]) -> Result<Vec<u8>, ImageRefusal> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > key.max_encoded_bytes() {
        return Err(ImageRefusal::TooLarge {
            cap: key.max_encoded_bytes(),
        });
    }
    let key_len = u32::try_from(key.cache_content_key.len())
        .map_err(|_| ImageRefusal::Transport("image cache key is too long".to_owned()))?;
    let capacity = CACHE_ENTRY_MAGIC
        .len()
        .checked_add(std::mem::size_of::<u32>())
        .and_then(|size| size.checked_add(key.cache_content_key.len()))
        .and_then(|size| size.checked_add(bytes.len()))
        .ok_or_else(|| ImageRefusal::Transport("image cache entry is too large".to_owned()))?;
    let mut entry = Vec::with_capacity(capacity);
    entry.extend_from_slice(CACHE_ENTRY_MAGIC);
    entry.extend_from_slice(&key_len.to_be_bytes());
    entry.extend_from_slice(key.cache_content_key.as_bytes());
    entry.extend_from_slice(bytes);
    Ok(entry)
}

fn decode_cache_entry<'a>(entry: &'a [u8], key: &ImageCacheKey) -> Option<&'a [u8]> {
    let length_end = CACHE_ENTRY_MAGIC
        .len()
        .checked_add(std::mem::size_of::<u32>())?;
    if entry.get(..CACHE_ENTRY_MAGIC.len())? != CACHE_ENTRY_MAGIC {
        return None;
    }
    let key_len = u32::from_be_bytes(
        entry
            .get(CACHE_ENTRY_MAGIC.len()..length_end)?
            .try_into()
            .ok()?,
    );
    let key_len = usize::try_from(key_len).ok()?;
    if key_len > MAX_CACHE_KEY_BYTES {
        return None;
    }
    let key_end = length_end.checked_add(key_len)?;
    if entry.get(length_end..key_end)? != key.cache_content_key.as_bytes() {
        return None;
    }
    let bytes = entry.get(key_end..)?;
    if u64::try_from(bytes.len()).ok()? > key.max_encoded_bytes() {
        return None;
    }
    Some(bytes)
}

fn cache_entry_read_limit(key: &ImageCacheKey) -> u64 {
    key.max_encoded_bytes()
        .saturating_add(u64::try_from(CACHE_ENTRY_MAGIC.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(std::mem::size_of::<u32>()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(MAX_CACHE_KEY_BYTES).unwrap_or(u64::MAX))
}

// A decoder cancellation maps to Cancelled, not Decode, so a superseded
// generation is never reported as a corrupt image.
fn map_decode_error(error: DecodeError) -> ImageRefusal {
    match error {
        DecodeError::Cancelled => ImageRefusal::Cancelled,
        other => ImageRefusal::Decode(other),
    }
}

// The decoder's own callback aborts a cancelled generation before the full
// decode and handle construction.
fn decode_kind(
    bytes: &[u8],
    kind: DecodeKind,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<DecodedAsset, ImageRefusal> {
    let limits = production_limits();
    let cancelled = || is_cancelled();
    match kind {
        DecodeKind::Raster => decode_static(bytes, &limits, cancelled),
        DecodeKind::Icon => decode_static_resized(bytes, &limits, 64, 64, cancelled),
    }
    .map_err(map_decode_error)
}

#[cfg(test)]
pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedAsset, ImageRefusal> {
    decode_kind(bytes, DecodeKind::Raster, &|| false)
}

#[cfg(test)]
fn commit_bytes(dest: &Path, bytes: &[u8]) -> Result<(), ImageRefusal> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ImageRefusal::Transport(e.to_string()))?;
    }
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| ImageRefusal::Transport(e.to_string()))?;
    if let Err(error) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(ImageRefusal::Transport(error.to_string()));
    }
    Ok(())
}

#[cfg(test)]
fn commit_raster_bytes_with(
    bytes: &[u8],
    dest: &Path,
    decode: fn(&[u8]) -> Result<DecodedAsset, ImageRefusal>,
) -> Result<RemoteImage, ImageRefusal> {
    let asset = decode(bytes)?;
    let image = RemoteImage::from_decoded(asset);
    commit_bytes(dest, bytes)?;
    Ok(image)
}

#[cfg(test)]
fn commit_raster_bytes(bytes: &[u8], dest: &Path) -> Result<RemoteImage, ImageRefusal> {
    commit_raster_bytes_with(bytes, dest, decode)
}

// Publication is a `renameat` beneath the held descriptor, so a pathname
// substitution of the cache directory cannot redirect it; a generation
// cancelled during or after decode publishes nothing.
fn commit_raster_bytes_at(
    bytes: &[u8],
    dir: &crate::roots::SecureDir,
    key: &ImageCacheKey,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<RemoteImage, ImageRefusal> {
    commit_raster_bytes_at_gated(bytes, dir, key, is_cancelled, is_cancelled)
}

fn commit_raster_bytes_at_gated(
    bytes: &[u8],
    dir: &crate::roots::SecureDir,
    key: &ImageCacheKey,
    decode_cancel: &dyn Fn() -> bool,
    publish_cancel: &dyn Fn() -> bool,
) -> Result<RemoteImage, ImageRefusal> {
    let asset = decode_kind(bytes, key.kind, decode_cancel)?;
    let image = RemoteImage::from_decoded(asset);
    let entry = encode_cache_entry(key, bytes)?;
    let published = dir
        .write_atomic_if(key.name(), &entry, || !publish_cancel())
        .map_err(|error| ImageRefusal::Transport(error.to_string()))?;
    if !published {
        return Err(ImageRefusal::Cancelled);
    }
    Ok(image)
}

// Absent, oversized, or content-key-mismatched entries are cache misses.
fn load_cached_at(
    dir: &crate::roots::SecureDir,
    key: &ImageCacheKey,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Option<RemoteImage>, ImageRefusal> {
    if is_cancelled() {
        return Err(ImageRefusal::Cancelled);
    }
    match dir.read_bounded(key.name(), cache_entry_read_limit(key)) {
        Ok(Some(entry)) => {
            let Some(bytes) = decode_cache_entry(&entry, key) else {
                return Ok(None);
            };
            decode_kind(bytes, key.kind, is_cancelled)
                .map(RemoteImage::from_decoded)
                .map(Some)
        }
        Ok(None) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Ok(None),
        Err(error) => Err(ImageRefusal::Transport(error.to_string())),
    }
}

pub(crate) fn load_cached_raster_at(
    dir: &crate::roots::SecureDir,
    key: &ImageCacheKey,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Option<RemoteImage>, ImageRefusal> {
    load_cached_at(dir, key, is_cancelled)
}

pub(crate) fn load_cached_icon_at(
    dir: &crate::roots::SecureDir,
    key: &ImageCacheKey,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Option<RemoteImage>, ImageRefusal> {
    load_cached_at(dir, key, is_cancelled)
}

#[cfg(test)]
pub(crate) fn load_cached(path: &Path) -> Result<RemoteImage, ImageRefusal> {
    let file = std::fs::File::open(path).map_err(|e| ImageRefusal::Transport(e.to_string()))?;
    let bytes = read_capped(file, MAX_DOWNLOAD_BYTES)?;
    decode(&bytes).map(RemoteImage::from_decoded)
}

// Completion-ordering seam: consume a capped body, require successful
// transport completion, then decode and commit. No destination or `.part`
// artifact exists while bytes are arriving or while completion is pending. It
// isolates the ordering guarantee for tests; the shipping cold route caps and
// completes inside the shared policy-safe transport before this decode/commit
// step runs.
#[cfg(test)]
pub(crate) fn commit_transport(
    reader: impl Read,
    completion: impl FnOnce() -> Result<(), ImageRefusal>,
    dest: &Path,
) -> Result<RemoteImage, ImageRefusal> {
    let bytes = read_capped(reader, MAX_DOWNLOAD_BYTES);
    // Completion is observed even after an over-cap/read refusal, so a truncated
    // or failed transfer never publishes a cache file.
    let completed = completion();
    let bytes = bytes?;
    completed?;
    commit_raster_bytes(&bytes, dest)
}

// The URL/address policy for a remote image fetch: HTTPS only, globally
// routable destinations only, and a bounded redirect chain. A `file://`,
// cleartext `http://`, credentialed, or private/link-local/non-global
// destination is refused before any connection, and the shared transport
// re-screens every DNS answer, redirect target, and connected peer under the
// same policy.
fn image_url_policy() -> UrlPolicy {
    UrlPolicy::DEFAULT
}

// The transport rejects the first byte beyond the per-kind ceiling.
fn image_transport_limits(max_bytes: u64) -> TransportLimits {
    TransportLimits {
        max_bytes,
        connect_timeout_ms: 10_000,
        read_timeout_ms: 15_000,
        total_timeout_ms: 20_000,
    }
}

// Translate a shared-transport failure into the app's typed refusal, keeping
// policy refusals (`Blocked`), cancellation, deadline, and over-cap outcomes
// distinct from a generic transport error.
fn refusal_from_transport(error: RemoteTransportError, max_bytes: u64) -> ImageRefusal {
    match error {
        RemoteTransportError::Url(verdict) => ImageRefusal::Blocked {
            reason: verdict.reason(),
        },
        RemoteTransportError::AddressBlocked(_) => ImageRefusal::Blocked {
            reason: "address-blocked",
        },
        RemoteTransportError::Cancelled(_) => ImageRefusal::Cancelled,
        RemoteTransportError::ByteLimit { .. } => ImageRefusal::TooLarge { cap: max_bytes },
        RemoteTransportError::Timeout => ImageRefusal::Timeout,
        RemoteTransportError::Dns => ImageRefusal::Transport("name resolution failed".to_owned()),
        RemoteTransportError::Redirect => ImageRefusal::Transport("invalid redirect".to_owned()),
        RemoteTransportError::HttpStatus(code) => {
            ImageRefusal::Transport(format!("HTTP status {code}"))
        }
        RemoteTransportError::ContentEncoding => {
            ImageRefusal::Transport("unsupported content encoding".to_owned())
        }
        RemoteTransportError::HeaderLimit(_) => {
            ImageRefusal::Transport("response header limit exceeded".to_owned())
        }
        RemoteTransportError::Backend(code) => {
            ImageRefusal::Transport(format!("transport backend error {code}"))
        }
        // `TransportError` is `#[non_exhaustive]`; a future variant is a
        // conservative transport refusal, never a silent success.
        _ => ImageRefusal::Transport("remote image transport failed".to_owned()),
    }
}

// Fetch one remote image body through the shared policy-safe transport under
// the production URL policy and image byte/deadline limits.
fn fetch_remote_image_bytes(
    url: &str,
    max_bytes: u64,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, ImageRefusal> {
    fetch_remote_bytes_with(
        url,
        &image_url_policy(),
        &image_transport_limits(max_bytes),
        is_cancelled,
    )
}

// Apply URL, peer, redirect, body and deadline limits while propagating cancellation into libcurl.
fn fetch_remote_bytes_with(
    url: &str,
    policy: &UrlPolicy,
    limits: &TransportLimits,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, ImageRefusal> {
    if is_cancelled() {
        return Err(ImageRefusal::Cancelled);
    }
    let request = RemoteRequest::checked(url, policy).map_err(|verdict| ImageRefusal::Blocked {
        reason: verdict.reason(),
    })?;
    let signal = CancelSignal::new();
    let probe = CancelProbe::new(&signal, is_cancelled, CancelCause::Cancelled);
    let mut progress = |_progress: TransferProgress| {};
    let response = CurlTransport::new()
        .fetch_with_cancel_probe(&request, policy, limits, &probe, &mut progress)
        .map_err(|error| refusal_from_transport(error, limits.max_bytes))?;
    // Defence in depth: the transport already refuses a body over the ceiling
    // while streaming, so a complete success is within the exact cap.
    if u64::try_from(response.body.len()).unwrap_or(u64::MAX) > limits.max_bytes {
        return Err(ImageRefusal::TooLarge {
            cap: limits.max_bytes,
        });
    }
    Ok(response.body)
}

fn fetch_image_at(
    dir: &crate::roots::SecureDir,
    key: &ImageCacheKey,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<RemoteImage, ImageRefusal> {
    let bytes =
        fetch_remote_image_bytes(&key.normalized_url, key.max_encoded_bytes(), is_cancelled)?;
    commit_raster_bytes_at(&bytes, dir, key, is_cancelled)
}

// The cache authority rejects publication after the key's detail generation expires.
pub(crate) fn fetch_screenshot_at(
    cache: &crate::roots::CacheCapability,
    key: &ImageCacheKey,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<RemoteImage, ImageRefusal> {
    let bytes =
        fetch_remote_image_bytes(&key.normalized_url, key.max_encoded_bytes(), is_cancelled)?;
    commit_detail_raster(&bytes, cache, key, is_cancelled)
}

fn commit_detail_raster(
    bytes: &[u8],
    cache: &crate::roots::CacheCapability,
    key: &ImageCacheKey,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<RemoteImage, ImageRefusal> {
    let asset = decode_kind(bytes, key.kind, is_cancelled)?;
    let image = RemoteImage::from_decoded(asset);
    let entry = encode_cache_entry(key, bytes)?;
    let generation = key.detail_generation.ok_or(ImageRefusal::Cancelled)?;
    let published = cache
        .publish_detail_image(key.name(), generation, &entry, is_cancelled)
        .map_err(|error| ImageRefusal::Transport(error.to_string()))?;
    if !published {
        return Err(ImageRefusal::Cancelled);
    }
    Ok(image)
}

// Download and commit one icon relative to the held cache descriptor.
//
// Icons are generation-owned blocking work: `is_cancelled` carries the owning
// catalog-load/refresh generation so a dropped or superseded catalog task
// aborts the in-flight icon fetch and publishes no cache file, rather than
// running each icon out to its total deadline.
pub(crate) fn fetch_icon_at(
    dir: &crate::roots::SecureDir,
    key: &ImageCacheKey,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<RemoteImage, ImageRefusal> {
    fetch_image_at(dir, key, is_cancelled)
}

#[cfg(test)]
mod tests;

/// Decode a local untrusted icon into bounded RGBA; no path reaches the renderer.
/// Rasterise an installed app's scalable icon into the same 64×64 RGBA form as
/// its raster icons. External and embedded images are never resolved, so the
/// parser reads nothing beyond `bytes`; no fonts are loaded, so text is not drawn.
/// The first embedded PNG or JPEG in an SVG tree, depth first.
fn embedded_raster(group: &resvg::usvg::Group) -> Option<&resvg::usvg::Image> {
    use resvg::usvg::{ImageKind, Node};
    group.children().iter().find_map(|node| match node {
        Node::Image(image) if matches!(image.kind(), ImageKind::PNG(_) | ImageKind::JPEG(_)) => {
            Some(image.as_ref())
        }
        Node::Group(group) => embedded_raster(group),
        _ => None,
    })
}

/// An embedded PNG, decoded only when its header declares a size an icon
/// can have (at most 1024 px a side), so a few bytes cannot ask for a huge
/// allocation.
fn bounded_png(data: &[u8]) -> Option<resvg::tiny_skia::Pixmap> {
    const MAX_SIDE: u32 = 1024;
    // The signature, then the IHDR chunk: length, type, width, height.
    if data.len() < 24 || &data[..8] != b"\x89PNG\r\n\x1a\n" || &data[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(data[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(data[20..24].try_into().ok()?);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    resvg::tiny_skia::Pixmap::decode_png(data).ok()
}

pub(crate) fn decode_local_svg_icon(
    bytes: &[u8],
    cancelled: &dyn Fn() -> bool,
) -> Option<RemoteImage> {
    use resvg::{tiny_skia, usvg};
    const SIDE: u32 = 64;
    if bytes.is_empty() || bytes.len() as u64 > MAX_ICON_DOWNLOAD_BYTES || cancelled() {
        return None;
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            // Embedded PNG and JPEG data are kept, not drawn (this build of
            // resvg draws no rasters): an icon that is only such a wrapper is
            // decoded below through the bounded icon decoder. Nothing outside
            // the file is ever read.
            resolve_data: Box::new(|mime, data, _| match mime {
                "image/png" => Some(usvg::ImageKind::PNG(data)),
                "image/jpeg" | "image/jpg" => Some(usvg::ImageKind::JPEG(data)),
                _ => None,
            }),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let size = tree.size();
    let scale = (SIDE as f32 / size.width()).min(SIDE as f32 / size.height());
    if !scale.is_finite() || scale <= 0.0 || cancelled() {
        return None;
    }
    let offset_x = (SIDE as f32 - size.width() * scale) / 2.0;
    let offset_y = (SIDE as f32 - size.height() * scale) / 2.0;
    let mut pixmap = tiny_skia::Pixmap::new(SIDE, SIDE)?;
    let transform =
        tiny_skia::Transform::from_scale(scale, scale).post_translate(offset_x, offset_y);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    if cancelled() {
        return None;
    }
    // An icon that drew nothing is usually a wrapper round an embedded
    // raster (Blanket's is): that raster, through the bounded decoder, is the
    // icon. One with no raster either is no icon, so the row falls back
    // instead of drawing blank.
    if pixmap.pixels().iter().all(|p| p.alpha() == 0) {
        let image = embedded_raster(tree.root())?;
        match image.kind() {
            // Drawn where the SVG places it, cropped to the icon's box.
            usvg::ImageKind::PNG(data) => {
                let raster = bounded_png(data)?;
                let size = image.size();
                let placement = transform.pre_concat(image.abs_transform()).pre_scale(
                    size.width() / raster.width() as f32,
                    size.height() / raster.height() as f32,
                );
                pixmap.draw_pixmap(
                    0,
                    0,
                    raster.as_ref(),
                    &tiny_skia::PixmapPaint {
                        quality: tiny_skia::FilterQuality::Bicubic,
                        ..Default::default()
                    },
                    placement,
                    None,
                );
                if pixmap.pixels().iter().all(|p| p.alpha() == 0) {
                    return None;
                }
            }
            // A JPEG has no transparent margins to crop: the picture whole.
            usvg::ImageKind::JPEG(data) => return decode_local_icon(data, cancelled).ok(),
            _ => return None,
        }
    }
    // tiny-skia stores premultiplied alpha; the image handle takes straight RGBA.
    let mut rgba = pixmap.take();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    Some(RemoteImage(Handle::from_rgba(SIDE, SIDE, rgba)))
}

pub(crate) fn decode_local_icon(
    bytes: &[u8],
    cancelled: &dyn Fn() -> bool,
) -> Result<RemoteImage, ImageRefusal> {
    let limits = DecodeLimits {
        max_encoded_bytes: MAX_ICON_DOWNLOAD_BYTES,
        max_width: 512,
        max_height: 512,
        max_pixels: 512 * 512,
        max_rgba_bytes: 512 * 512 * 4,
    };
    decode_static_resized(bytes, &limits, 64, 64, cancelled)
        .map(RemoteImage::from_decoded)
        .map_err(map_decode_error)
}
