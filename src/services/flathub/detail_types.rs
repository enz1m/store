// Detail route state and context.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum DetailRouteState {
    #[default]
    Loading,
    Known,
    KnownEmpty,
    Failed,
    CachedStale {
        age_secs: u64,
    },
}

/// A single bundle of "trust" data used by the detail-page context bar.
/// Each field is independently populated so a partial failure (e.g. the
/// verification endpoint 500ing while stats succeed) still produces a
/// useful bar. Callers wire an `Option<DetailContext>` through the model
/// with the pending flag distinct from "loaded but empty".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DetailContext {
    pub metadata_state: DetailRouteState,
    pub stats_state: DetailRouteState,
    pub ratings_state: DetailRouteState,
    pub description: String,
    /// SPDX license expression from `project_license`; empty when unknown.
    pub license: String,
    /// Classification of `license`: FOSS / Proprietary / Unknown.
    pub license_kind: LicenseKind,
    /// Publisher verification preserves answered, retained and unknown states.
    pub verification: PublisherVerification,
    /// installs_last_month from the stats endpoint; 0 when unknown.
    pub installs_last_month: u64,
    /// Human-readable age rating ("12+", "17+", ...); empty when unknown.
    pub age_rating: String,
    /// Raw OARS content-rating map from the appstream JSON. Kept so the
    /// detail dialog can show a per-category breakdown without re-fetching.
    pub content_rating: std::collections::HashMap<String, String>,
    /// Release history from the appstream JSON. Empty when the app doesn't
    /// ship a `<releases>` block. Ordered newest-first.
    pub releases: Vec<FlathubRelease>,
    /// End-of-life signal from the appstream JSON. `is_eol` mirrors
    /// whether Flathub marked the app as EOL (`end_of_life` /
    /// `is_eol_message` non-empty); `eol_message` carries the reason text
    /// so a future detail-page banner can render "This app is no longer
    /// maintained: `<reason>`" honestly.
    pub is_eol: bool,
    pub eol_message: String,
    /// Canonical URLs the publisher declared for the app — homepage,
    /// bug tracker, donation, help. Empty when the appstream endpoint
    /// omitted the `urls` block or every user-visible slot was null.
    /// The detail page's "Links" row renders one ghost button per
    /// non-empty URL, dispatched through the desktop's link handler
    /// (`crate::url_launch::UrlLauncher`).
    pub urls: FlathubUrls,
    /// Pre-install download size in bytes. 0 when unknown. Fetched
    /// from `flatpak remote-info --show-commit` or inferred from
    /// the installed size field. Rendered as a human-readable tile
    /// (e.g. "42 MB") in the detail-page context bar.
    pub download_size: u64,
    /// Human-readable "last updated" date. Derived from the newest
    /// release timestamp in the appstream data (we already parse
    /// `FlathubRelease.formatted_date()`). Empty when the app has
    /// no releases metadata.
    pub last_updated: String,
    /// The selected app's ODRS histogram, cached by app ID and catalogue URL.
    pub ratings: Option<RatingsCounts>,
    /// How many screenshots the appstream endpoint reported for this app.
    pub screenshot_count: usize,
    // Validated URL candidates transferred once to the generation-bound image
    // task; the rendered context retains no second copy.
    pub(crate) screenshot_candidates: Vec<FlathubScreenshot>,
    /// The owning read task was dropped part-way through the enrichment
    /// fan-out: the fields above hold whatever completed before the stop and
    /// the rest are defaults. A live consumer never observes this — the flag
    /// is only ever set after that consumer asked for teardown — but it keeps
    /// the partial value a defined result rather than an indistinguishable
    /// "everything failed" context.
    pub cancelled: bool,
}

impl DetailContext {
    #[must_use]
    pub(crate) fn failed() -> Self {
        Self {
            metadata_state: DetailRouteState::Failed,
            stats_state: DetailRouteState::Failed,
            ratings_state: DetailRouteState::Failed,
            verification: PublisherVerification::unavailable(VerificationFailure::Transport(
                "detail worker unavailable".to_owned(),
            )),
            ..Self::default()
        }
    }

    #[must_use]
    fn unsupported_origin() -> Self {
        Self {
            metadata_state: DetailRouteState::Failed,
            stats_state: DetailRouteState::Failed,
            ratings_state: DetailRouteState::Failed,
            verification: PublisherVerification::unavailable(VerificationFailure::Policy),
            ..Self::default()
        }
    }
}
