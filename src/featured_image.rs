//! Bounded featured screenshot loading on the catalog read pool.
//!
//! Resolve artwork through the per-app AppStream endpoint and decode it through
//! the framework image service. The dedicated HTTP cache revalidates responses
//! and evicts old entries within HERO_CACHE_BYTES.

use enzui_core::async_image::{CompletionOutcome, Generation, ImageState};
use enzui_resources::curl_transport::CurlTransport;
use enzui_resources::{
    HttpCacheConfig, ImageService, RequestDisposition, ServiceConfig, ServiceEvent, Transport,
};
use iced::widget::image::Handle;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Total encoded screenshot bytes retained on disk.
pub(crate) const HERO_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// A single screenshot may consume at most one eighth of the cache.
pub(crate) const HERO_CACHE_ENTRY_BYTES: u64 = 8 * 1024 * 1024;

/// One hero's render lifecycle.
#[derive(Clone, Debug)]
pub(crate) enum HeroImageState {
    Loading,
    Ready(Handle),
    Unavailable(String),
}

impl HeroImageState {
    /// Convert to the framework widget's pure lifecycle snapshot.
    pub(crate) fn image_state(&self) -> ImageState<Handle> {
        match self {
            Self::Loading => ImageState::Loading(Generation::FIRST),
            Self::Ready(handle) => ImageState::Ready(handle.clone()),
            Self::Unavailable(reason) => ImageState::Failed(
                enzui_core::async_image::FailureReason::Service(reason.clone()),
            ),
        }
    }
}

/// A background result, keyed by app id so a late page cannot overwrite the
/// currently selected hero.
///
/// `result` is the single hero screenshot and owns the page's loading and
/// failure lifecycle. The Flathub-style hero shows exactly one screenshot, so
/// no companion shots are fetched or decoded.
#[derive(Clone, Debug)]
pub(crate) struct HeroImageOutcome {
    pub app_id: String,
    /// The publisher's primary colour for the light and the dark scheme,
    /// when the app declares one; the banner falls back without it.
    pub brand: Option<([u8; 3], [u8; 3])>,
    pub result: Result<Handle, String>,
}

/// Construct the framework cache policy. Kept as a function so app tests can
/// exercise eviction at a tiny budget through the exact production wiring.
pub(crate) fn http_cache_config(max_bytes: u64, max_entry_bytes: u64) -> HttpCacheConfig {
    HttpCacheConfig {
        max_bytes,
        max_entry_bytes,
    }
}

/// The service policy used by the shipping loader and the deterministic tests.
pub(crate) fn service_config() -> ServiceConfig {
    ServiceConfig {
        decode_limits: crate::bounded_image::production_limits(),
        worker_threads: 1,
        decoded_cache_bytes: 0,
        max_inflight_jobs: 1,
        max_inflight_request_bytes: 64 * 1024,
        http_cache: http_cache_config(HERO_CACHE_BYTES, HERO_CACHE_ENTRY_BYTES),
        ..ServiceConfig::default()
    }
}

/// Blocking worker body. Call only from a bounded read-pool submission, never
/// directly inside a task's async body.
///
/// The caller passes the validated featured-image cache directory from the
/// captured cache capability; a store with no cache tier schedules no hero load
/// rather than writing to an arbitrary or relative location.
pub(crate) fn load(cache_root: PathBuf, app_id: String) -> HeroImageOutcome {
    let (brand, result) = load_inner(&cache_root, &app_id);
    HeroImageOutcome {
        app_id,
        brand,
        result,
    }
}

/// The screenshot width the banner asks for.
const BANNER_SHOT_WIDTH: i32 = 1248;

pub(crate) fn stage_shot_url(info: &crate::services::FlathubAppInfo) -> Option<String> {
    info.screenshots
        .iter()
        .find_map(|shot| {
            // The banner shows a screenshot about 600 px wide: twice that
            // stays sharp on a scaled display.
            shot.url_near(BANNER_SHOT_WIDTH)
                .or_else(|| shot.thumb_url())
                .filter(|url| !url.is_empty())
        })
        .map(str::to_owned)
}

// The brand colours come from the same AppStream record as the screenshot
// URL, so a banner keeps its colour even when the screenshot fails.
type BrandColors = Option<([u8; 3], [u8; 3])>;

fn load_inner(cache_root: &Path, app_id: &str) -> (BrandColors, Result<Handle, String>) {
    // The id becomes one path segment of a fixed Flathub endpoint. Use the
    // same Flatpak application-ID parser as catalogue ingestion and process
    // effects rather than a route-local character allowlist.
    if let Err(error) = crate::flatpak_args::validate_app_id(app_id) {
        return (None, Err(error));
    }
    let Ok(info) = crate::services::flathub_app_info(app_id) else {
        return (None, Err("screenshot metadata unavailable".to_owned()));
    };
    let brand = info.brand_colors();
    let Some(url) = stage_shot_url(&info) else {
        return (brand, Err("no screenshot supplied".to_owned()));
    };
    let loaded = load_url_with(
        app_id,
        &url,
        CurlTransport::new(),
        service_config(),
        cache_root,
    );
    (brand, loaded.result.map(trim_transparent_margins))
}

/// A screenshot without the transparent border some ship around a
/// window (Firefox's), so the window itself fills the banner's slot.
/// Anything not decoded to pixels, or without such a border, is unchanged.
pub(crate) fn trim_transparent_margins(handle: Handle) -> Handle {
    let Handle::Rgba {
        width,
        height,
        ref pixels,
        ..
    } = handle
    else {
        return handle;
    };
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || pixels.len() < w * h * 4 {
        return handle;
    }
    // Nearly clear pixels are border too: shadows fade to zero.
    let solid = |x: usize, y: usize| pixels[(y * w + x) * 4 + 3] > 24;
    let row_solid = |y: usize| (0..w).any(|x| solid(x, y));
    let col_solid = |x: usize, top: usize, bottom: usize| (top..=bottom).any(|y| solid(x, y));
    let Some(top) = (0..h).find(|&y| row_solid(y)) else {
        return handle;
    };
    let bottom = (0..h).rev().find(|&y| row_solid(y)).unwrap_or(top);
    let left = (0..w).find(|&x| col_solid(x, top, bottom)).unwrap_or(0);
    let right = (0..w)
        .rev()
        .find(|&x| col_solid(x, top, bottom))
        .unwrap_or(w - 1);
    let (new_w, new_h) = (right - left + 1, bottom - top + 1);
    // Leave a screenshot whose border is a sliver alone.
    if new_w * 100 >= w * 98 && new_h * 100 >= h * 98 {
        return handle;
    }
    let mut cropped = Vec::with_capacity(new_w * new_h * 4);
    for y in top..=bottom {
        let start = (y * w + left) * 4;
        cropped.extend_from_slice(&pixels[start..start + new_w * 4]);
    }
    Handle::from_rgba(new_w as u32, new_h as u32, cropped)
}

// Load one already-resolved URL through the real service. Public within the
// crate so loopback tests can inject a counted transport/config while still
// exercising the production cache/decode path.
pub(crate) fn load_url_with<T: Transport>(
    app_id: &str,
    url: &str,
    transport: T,
    config: ServiceConfig,
    cache_root: &Path,
) -> HeroImageOutcome {
    let mut service = match ImageService::new_with_cache_root(transport, config, cache_root) {
        Ok(service) => service,
        Err(error) => {
            return HeroImageOutcome {
                app_id: app_id.to_owned(),
                brand: None,
                result: Err(error.to_string()),
            };
        }
    };
    let generation = Generation::FIRST.next();
    match service.request(url, generation) {
        Ok(RequestDisposition::Busy(_)) => {
            return HeroImageOutcome {
                app_id: app_id.to_owned(),
                brand: None,
                result: Err("image service busy".into()),
            };
        }
        Ok(_) => {}
        Err(verdict) => {
            return HeroImageOutcome {
                app_id: app_id.to_owned(),
                brand: None,
                result: Err(format!("URL refused: {}", verdict.reason())),
            };
        }
    }

    // Transport owns the actual deadline. This outer deadline is only a guard
    // against a broken worker/channel leaving the Task pending forever.
    let guard = Duration::from_millis(
        config
            .transport_limits
            .total_timeout_ms
            .saturating_add(1000),
    );
    let started = Instant::now();
    loop {
        for event in service.poll() {
            match event {
                ServiceEvent::Ready { completion, .. } => {
                    let result = match completion.outcome {
                        CompletionOutcome::Decoded { handle, .. } => Ok(handle),
                        CompletionOutcome::Failed(error) => Err(error),
                    };
                    return HeroImageOutcome {
                        app_id: app_id.to_owned(),
                        brand: None,
                        result,
                    };
                }
                ServiceEvent::Failed { error, .. } => {
                    return HeroImageOutcome {
                        app_id: app_id.to_owned(),
                        brand: None,
                        result: Err(error.to_string()),
                    };
                }
                ServiceEvent::Cancelled { .. } => {
                    return HeroImageOutcome {
                        app_id: app_id.to_owned(),
                        brand: None,
                        result: Err("image request cancelled".into()),
                    };
                }
                ServiceEvent::Progress { .. } => {}
            }
        }
        if started.elapsed() >= guard {
            return HeroImageOutcome {
                app_id: app_id.to_owned(),
                brand: None,
                result: Err("image request timed out".into()),
            };
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

// Unknown fixture selectors fail loudly instead of concealing broken-image cases.
pub(crate) fn fixture_hero_state(value: Option<&str>) -> HeroImageState {
    match value {
        None | Some("image") => HeroImageState::Ready(fixture_handle()),
        Some("offline") => HeroImageState::Unavailable("offline".into()),
        Some(other) => panic!(
            "ENZUI_FIXTURE_HERO={other:?} is not a recognised fixture hero \
             value; expected unset, \"image\" or \"offline\""
        ),
    }
}

/// Returns deterministic landscape pixels for capture fixtures.
pub(crate) fn fixture_handle() -> Handle {
    fixture_handle_variant(0)
}

fn fixture_handle_variant(variant: usize) -> Handle {
    let (w, h): (u32, u32) = match variant % 3 {
        0 => (960, 540),
        1 => (720, 900),
        _ => (900, 600),
    };
    let tint = |base: (u8, u8, u8)| -> (u8, u8, u8) {
        let shift = (variant as u8) * 26;
        (
            base.0.saturating_add(shift),
            base.1.saturating_sub(shift / 3),
            base.2.saturating_add(shift / 2),
        )
    };
    let title_band = h / 9;
    let rail = w / 5;
    let mut rgba = vec![0_u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let band = |lo: u32, hi: u32| (y * 100 / h) >= lo && (y * 100 / h) < hi;
            let content = x > rail + w / 20;
            let (r, g, b) = if y < title_band {
                tint((36, 42, 58))
            } else if x < rail {
                tint((46, 54, 72))
            } else if content && band(18, 30) {
                tint((97, 113, 164))
            } else if content && band(35, 47) {
                tint((74, 143, 126))
            } else if content && band(52, 84) {
                tint((225, 158, 77))
            } else {
                (236, 239, 247)
            };
            let i = ((y * w + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    Handle::from_rgba(w, h, rgba)
}
