// Flathub app-info types and release formatting.

// ---- Flathub REST API -----------------------------------------------------

use enzui_core::cancellation::CancelCause;
use enzui_core::url_policy::{AddressClass, Scheme, TransportLimits, UrlPolicy, UrlVerdict};
use enzui_resources::curl_transport::{CancelProbe, CurlTransport};
use enzui_resources::header_limits::HeaderLimitKind;
use enzui_resources::{
    CacheIdentityDomain, CancelSignal, RemoteRequest, TransferProgress,
    TransportError as RemoteTransportError, TransportResponse, cache_identity,
};

/// A single hit from the Flathub collection API.
/// The v2 collection endpoint returns `{"hits": [...]}` where each hit has
/// metadata directly (no per-app fetch needed for basic card data).
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct FlathubCollectionHit {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub developer_name: String,
    #[serde(default)]
    #[allow(dead_code, reason = "Retain the deserialized upstream API wire shape")]
    pub project_license: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub icon: String,
    #[serde(default)]
    pub verification_verified: bool,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    pub installs_last_month: u64,
    // Flathub category values may be strings, arrays or null; one atypical value must not reject the catalog.
    #[serde(default, deserialize_with = "deserialize_category")]
    pub main_categories: String,
}

// Accept `"game"`, `[]`, `["game", "utility"]` or `null` for a category.
//
// A non-empty array takes its first element: Flathub's own UI shows one
// primary category, and the field is named `main_categories` because the
// upstream schema is a list even though it almost always carries one value.
fn deserialize_category<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;

    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Category {
        One(String),
        Many(Vec<String>),
        Null,
    }
    Ok(match Category::deserialize(deserializer)? {
        Category::One(s) => s,
        Category::Many(v) => v.into_iter().next().unwrap_or_default(),
        Category::Null => String::new(),
    })
}

// The collection envelope carries pagination fields needed to bound complete catalogue ingestion.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct FlathubCollectionResponse {
    #[serde(default)]
    pub hits: Vec<FlathubCollectionHit>,
    #[serde(default = "one", rename = "page")]
    pub page: u32,
    #[serde(default = "one", rename = "totalPages")]
    pub total_pages: u32,
    #[serde(default, rename = "totalHits")]
    pub total_hits: u32,
}

fn one() -> u32 {
    1
}

// Keep the complete wire payload even when catalogue metadata supplies the displayed fields.
#[allow(dead_code, reason = "Retain the deserialized upstream API wire shape")]
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct FlathubAppInfo {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub summary: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub description: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub developer_name: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub project_license: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub icon: String,
    #[serde(default)]
    pub screenshots: Vec<FlathubScreenshot>,
    /// OARS content-rating map, e.g. `{"oars-1.1": "moderate", "violence-cartoon": "mild"}`.
    /// The single highest severity level is what we surface as an age tile.
    #[serde(default)]
    pub content_rating: std::collections::HashMap<String, String>,
    /// Release history from the appstream metadata. Ordered newest-first by
    /// Flathub; we preserve that ordering. Empty when the app has no
    /// `<releases>` block, which is common for older packages.
    #[serde(default)]
    pub releases: Vec<FlathubRelease>,
    /// End-of-life marker string (non-empty when Flathub has marked the app
    /// as EOL, empty otherwise). We surface presence only — the reason
    /// isn't rendered directly, but its non-emptiness feeds the Search
    /// view's "Hide EOL" filter and could power a detail-page banner.
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub end_of_life: String,
    /// Newer Flathub API responses use `is_eol_message` instead of the older
    /// `end_of_life`. Accepted as an alias so both wire formats populate the
    /// same signal.
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub is_eol_message: String,
    /// Publisher-declared canonical URLs for the app. Flathub returns
    /// a map with keys `homepage`, `bugtracker`, `donation`, `help`,
    /// `translate`, `faq`, `contact`, `vcs_browser`, `contribute`.
    /// Any of them may be `null` — we accept null via
    /// `serde(default)` in [`FlathubUrls`], which resolves to an empty
    /// string. Wired into the M53 detail-page "Links" row.
    #[serde(default)]
    pub urls: FlathubUrls,
    /// The publisher's brand colours, one per colour scheme: what Flathub
    /// paints an app's banner with.
    /// Many apps send `null` here, which is no branding, not a bad record.
    #[serde(default, deserialize_with = "deserialize_null_as_empty")]
    pub branding: Vec<FlathubBrandColor>,
}

/// One AppStream `<branding><color>`: `{"value": "#bef8a5", "type":
/// "primary", "scheme_preference": "light"}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub(crate) struct FlathubBrandColor {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub value: String,
    #[serde(default, rename = "type", deserialize_with = "deserialize_optional_string")]
    pub kind: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub scheme_preference: String,
}

impl FlathubAppInfo {
    /// The primary brand colour for the light and the dark scheme, as
    /// `#rrggbb` parsed to RGB. A colour without a scheme serves both; a
    /// missing scheme borrows the other's.
    #[must_use]
    pub(crate) fn brand_colors(&self) -> Option<([u8; 3], [u8; 3])> {
        let pick = |scheme: &str| {
            self.branding
                .iter()
                .filter(|c| c.kind.is_empty() || c.kind == "primary")
                .find(|c| c.scheme_preference == scheme)
                .and_then(|c| parse_hex_rgb(&c.value))
        };
        let any = pick("");
        let light = pick("light").or(any);
        let dark = pick("dark").or(any);
        match (light, dark) {
            (Some(l), Some(d)) => Some((l, d)),
            (Some(c), None) | (None, Some(c)) => Some((c, c)),
            (None, None) => None,
        }
    }
}

/// `#rrggbb` (or `#rgb`) to RGB; anything else is no colour.
fn parse_hex_rgb(value: &str) -> Option<[u8; 3]> {
    let hex = value.trim().strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        6 => Some([channel(&hex[0..2])?, channel(&hex[2..4])?, channel(&hex[4..6])?]),
        3 => {
            let c = |i: usize| channel(&hex[i..=i]).map(|v| v * 17);
            Some([c(0)?, c(1)?, c(2)?])
        }
        _ => None,
    }
}

/// URL map from the Flathub per-app appstream `urls` field. Every key
/// is optional in the wire format — `serde(default)` gives us an empty
/// string when a URL is null or absent, so the UI can gate each button
/// on `!s.is_empty()` without unwrapping.
///
/// We only surface the four keys with a meaningful in-app action
/// (Homepage / Bug tracker / Donation / Help). The remaining fields
/// (`vcs_browser`, `contribute`, ...) are parsed for future use but
/// not rendered.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(crate) struct FlathubUrls {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub homepage: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub bugtracker: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub donation: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub help: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub vcs_browser: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub contribute: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub translate: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub faq: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub contact: String,
}

impl FlathubUrls {
    /// Returns true when every user-visible URL slot is empty. Used by
    /// the UI to hide the entire "Links" row so it doesn't leave a
    /// phantom label above no buttons.
    pub(crate) fn is_empty(&self) -> bool {
        self.homepage.is_empty()
            && self.bugtracker.is_empty()
            && self.donation.is_empty()
            && self.help.is_empty()
    }
}

// serde helper: accept either a string, `null`, or absent field, and
// return `String::new()` for the null/absent cases. Flathub's `urls`
// map emits explicit `null` for missing entries (rather than omitting
// the key), which serde's default `Option<String>` handles fine but
// leaves the model with `None` vs `Some("")` ambiguity. Collapsing to
// a single empty-string sentinel keeps downstream checks trivial.
// A list the API may send as `null`: read as empty.
fn deserialize_null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    use serde::Deserialize;
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let opt: Option<String> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

fn deserialize_optional_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let opt: Option<u64> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// One `<release>` entry from the appstream JSON. Flathub's API sometimes
/// emits the timestamp as a Unix-seconds integer, sometimes as an ISO date
/// string — both are captured as strings and normalised at render time.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub(crate) struct FlathubRelease {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub version: String,
    /// Raw timestamp as returned by Flathub. May be a Unix seconds integer
    /// ("1725926400"), an ISO date ("2024-09-09"), or an empty string.
    #[serde(default, deserialize_with = "deserialize_timestamp")]
    pub timestamp: String,
    /// Release notes body. Often HTML wrapped in `<p>` / `<ul>`; the UI
    /// strips tags for display. Flathub emits `null` when a release has no
    /// notes (including Firefox's current appstream document).
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub description: String,
    /// `"stable"` / `"development"` per appstream schema; empty when unset.
    #[serde(
        default,
        rename = "type",
        deserialize_with = "deserialize_optional_string"
    )]
    pub release_type: String,
}

// Accept both string and integer JSON values for the `timestamp` field.
fn deserialize_timestamp<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use serde::de::{Deserialize, Error};
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => {
            return Err(D::Error::custom(format!(
                "expected timestamp string or int, got {other:?}"
            )));
        }
    })
}

impl FlathubRelease {
    /// Human-readable release date. Falls back to the raw timestamp string
    /// on any parse failure so the value is at least legible — we never want
    /// to render an empty date next to a version tag.
    pub(crate) fn formatted_date(&self) -> String {
        format_release_timestamp(&self.timestamp)
    }

    /// Plain-text preview of the description, HTML tags stripped and
    /// paragraph breaks preserved as blank lines.
    pub(crate) fn plain_description(&self) -> String {
        strip_html(&self.description)
    }
}

/// Convert Flathub's mixed-format timestamp into a human-readable date
/// string like `"Jul 18, 2026"`. Handles pure Unix seconds and ISO 8601
/// (`YYYY-MM-DD[Thh:mm:ss...]`).
pub(crate) fn format_release_timestamp(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Unix seconds path: pure digits.
    if trimmed.chars().all(|c| c.is_ascii_digit())
        && let Ok(secs) = trimmed.parse::<i64>()
    {
        return unix_secs_to_human(secs);
    }
    // ISO path: parse the leading `YYYY-MM-DD` and reformat.
    // Byte slicing below is only valid for ASCII text (a multibyte character
    // in the first ten bytes would panic the UI thread).
    if trimmed.len() >= 10
        && trimmed.is_char_boundary(10)
        && trimmed[..10].is_ascii()
        && trimmed[..4].chars().all(|c| c.is_ascii_digit())
        && &trimmed[4..5] == "-"
        && trimmed[5..7].chars().all(|c| c.is_ascii_digit())
        && &trimmed[7..8] == "-"
        && trimmed[8..10].chars().all(|c| c.is_ascii_digit())
    {
        let y: i64 = trimmed[..4].parse().unwrap_or(0);
        let m: u32 = trimmed[5..7].parse().unwrap_or(0);
        let d: u32 = trimmed[8..10].parse().unwrap_or(0);
        return format_ymd(y, m, d);
    }
    trimmed.to_owned()
}

/// Month abbreviations (1-indexed; index 0 unused).
const MONTH_ABBREV: [&str; 13] = [
    "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// Format year/month/day into `"Mon DD, YYYY"` (e.g. `"Jul 18, 2026"`).
fn format_ymd(y: i64, m: u32, d: u32) -> String {
    let month_name = if (1..=12).contains(&m) {
        MONTH_ABBREV[m as usize]
    } else {
        "???"
    };
    format!("{month_name} {d}, {y}")
}

// Convert Unix seconds to a human-readable date like `"Jul 18, 2026"` in UTC.
// Zero external deps — uses Howard Hinnant's civil_from_days algorithm.
fn unix_secs_to_human(secs: i64) -> String {
    // Days since 1970-01-01 (floor division).
    let days = secs.div_euclid(86_400);
    // Shift epoch to 0000-03-01 for arithmetic; add 719_468.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // 0..=146_096
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // 0..=399
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // 0..=365
    let mp = (5 * doy + 2) / 153; // 0..=11
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // day: 1..=31
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // month: 1..=12
    let y = if m <= 2 { y + 1 } else { y };
    format_ymd(y, m, d)
}

/// Strip HTML tags for a plain-text release-notes preview. Preserves visible
/// text, collapses runs of whitespace, and turns paragraph-breaks into
/// double-newlines. This is not a real HTML parser — but Flathub's release
/// notes are simple `<p>`/`<ul>`/`<li>` snippets in practice.
pub(crate) fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    // Track a rough "pending break" so `<p>foo</p><p>bar</p>` renders as two
    // paragraphs (single newline between them, not three).
    let mut pending_break = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c == '>' {
            in_tag = false;
            pending_break = true;
            continue;
        }
        if in_tag {
            continue;
        }
        if pending_break {
            // Leading whitespace after a tag must not consume a pending paragraph break.
            if c.is_whitespace() {
                continue;
            }
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            pending_break = false;
        }
        if c.is_whitespace() {
            if !out.ends_with(' ') && !out.ends_with('\n') && !out.is_empty() {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    // Decode a few very common HTML entities.
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    out.trim().to_owned()
}

#[cfg(test)]
mod app_info_tests {
    use super::*;

    #[test]
    fn an_app_without_branding_still_reads() {
        // Firefox's and Chrome's records send `"branding": null`.
        let info: FlathubAppInfo =
            serde_json::from_str(r#"{"name": "Firefox", "branding": null}"#).unwrap();
        assert_eq!(info.name, "Firefox");
        assert_eq!(info.brand_colors(), None);
    }

}
