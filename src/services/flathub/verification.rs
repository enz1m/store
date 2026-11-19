// Publisher verification types and status.

/// Publisher verification distinguishes confirmed status from an unavailable answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PublisherVerification {
    /// The verification endpoint answered: this publisher is verified.
    Verified {
        /// How Flathub verified them ("website", "login_provider", ...).
        /// Empty when the endpoint did not say.
        method: String,
        /// Where this answer came from, and how old it is.
        provenance: VerificationProvenance,
    },
    /// The verification endpoint answered: this publisher is **not** verified.
    /// A real negative, not a missing answer.
    KnownUnverified {
        /// Where this answer came from, and how old it is.
        provenance: VerificationProvenance,
    },
    /// No answer. Carries why, and who may act on it.
    Unknown(VerificationUnavailable),
}

impl Default for PublisherVerification {
    // An unenriched context knows nothing, and says so.
    //
    // Deliberately not `KnownUnverified`: `DetailContext::default()` is what
    // a detail page holds before enrichment lands, and defaulting to a
    // negative trust claim is the defect in a different costume.
    fn default() -> Self {
        Self::pending()
    }
}

impl PublisherVerification {
    /// The state a detail page holds while enrichment is still in flight.
    #[must_use]
    pub(crate) fn pending() -> Self {
        Self::Unknown(VerificationUnavailable {
            reason: VerificationFailure::Pending,
            retry: RetryOwner::PendingLoad,
        })
    }

    /// An unknown state for a stated typed reason.
    #[must_use]
    pub(crate) fn unavailable(reason: VerificationFailure) -> Self {
        let retry = reason.retry_owner();
        Self::Unknown(VerificationUnavailable { reason, retry })
    }

    /// Whether this publisher is **known to be** verified.
    ///
    /// The only question a trust decision may ask. `Unknown` answers `false`
    /// here *and* `false` to [`Self::is_known_unverified`], which is the whole
    /// point: absence of an answer is not an answer either way, so a
    /// verified-only filter cannot admit it and unverified styling cannot
    /// claim it.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }

    /// Whether an answer was obtained at all.
    #[must_use]
    pub(crate) fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }

    /// The failure detail, when there is no answer.
    #[must_use]
    pub(crate) fn unavailable_reason(&self) -> Option<&VerificationUnavailable> {
        match self {
            Self::Unknown(unavailable) => Some(unavailable),
            _ => None,
        }
    }

    /// When this answer was obtained, when there is one.
    #[must_use]
    pub(crate) fn provenance(&self) -> Option<&VerificationProvenance> {
        match self {
            Self::Verified { provenance, .. } | Self::KnownUnverified { provenance } => {
                Some(provenance)
            }
            Self::Unknown(_) => None,
        }
    }

    /// Retain only a known answer with an explicit age; unknown cannot become an answered verdict.
    #[must_use]
    pub(crate) fn retained(&self, age_ms: u64) -> Option<Self> {
        let provenance = VerificationProvenance::Retained { age_ms };
        match self {
            Self::Verified { method, .. } => Some(Self::Verified {
                method: method.clone(),
                provenance,
            }),
            Self::KnownUnverified { .. } => Some(Self::KnownUnverified { provenance }),
            Self::Unknown(_) => None,
        }
    }
}

/// Cached and retained verification must disclose age; answers without provenance become Unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VerificationProvenance {
    /// The verification endpoint answered during this load. No age to state.
    Live,
    /// A known answer from earlier in this session, kept because a later
    /// attempt failed. The age travels with it and is disclosed.
    Retained {
        /// Milliseconds since the answer was obtained.
        age_ms: u64,
    },
}

impl VerificationProvenance {
    /// The disclosure suffix for a retained answer, or `None` when live.
    #[must_use]
    pub(crate) fn staleness(&self) -> Option<String> {
        match self {
            Self::Live => None,
            Self::Retained { age_ms } => Some(format!(
                "Checked {} ago; the latest check failed.",
                crate::model::format_age(*age_ms)
            )),
        }
    }
}

/// The unavailable-verification reason and ownership of any retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerificationUnavailable {
    /// The typed reason, rendered as a bounded disclosure.
    pub reason: VerificationFailure,
    /// Whether a user-visible Retry may be offered.
    pub retry: RetryOwner,
}

/// Distinguish retryable failures from invalid or superseded requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VerificationFailure {
    /// Enrichment has not answered yet for this app.
    Pending,
    /// The request was never made: the app id is not a valid Flatpak id, so
    /// there is no endpoint to ask.
    NotRequested,
    /// The owner went away mid-request — the user navigated to another app.
    /// A newer load owns the answer, so this one must not be retried or
    /// published.
    Cancelled,
    /// Destination policy refused the request. Repeating the same request
    /// cannot make that destination admissible.
    Policy,
    /// The transport failed: no network, a deadline, or an oversized body.
    Transport(String),
    /// The endpoint was reached and answered unsuccessfully.
    Status(String),
    /// The endpoint answered successfully with a body that is not the
    /// documented schema.
    Schema(String),
}

impl VerificationFailure {
    // Who may act on this failure.
    fn retry_owner(&self) -> RetryOwner {
        match self {
            // A load is already running, or about to; a second one would race
            // it and could publish the older answer last.
            Self::Pending | Self::Cancelled => RetryOwner::PendingLoad,
            // There is no admissible request to repeat.
            Self::NotRequested | Self::Policy => RetryOwner::Unretryable,
            // A repeat may genuinely succeed: the network, the service, or a
            // mid-deploy body can all recover.
            Self::Transport(_) | Self::Status(_) | Self::Schema(_) => RetryOwner::User,
        }
    }

    /// The bounded, user-facing sentence for this failure.
    ///
    /// Bounded at the shared rendered-failure cap, because the transport and
    /// status details are external text: a service returning a megabyte of
    /// HTML must not become a megabyte of accessible name.
    #[must_use]
    pub(crate) fn disclosure(&self) -> String {
        let sentence = match self {
            Self::Pending => "Checking Flathub for publisher verification.".to_owned(),
            Self::NotRequested => {
                "Flathub was not asked: this app id is not a valid Flatpak id.".to_owned()
            }
            Self::Cancelled => "The check stopped when you moved to another app.".to_owned(),
            Self::Policy => {
                "Flathub was not asked because network safety policy refused the destination."
                    .to_owned()
            }
            Self::Transport(detail) => {
                format!("Could not reach Flathub to check verification: {detail}")
            }
            Self::Status(detail) => {
                format!("Flathub could not answer the verification check: {detail}")
            }
            Self::Schema(_) => "Flathub's verification answer could not be read.".to_owned(),
        };
        crate::services::flatpak_failure::cap_chars(
            &crate::services::flatpak_failure::sanitize_diagnostic(&sentence),
            VERIFICATION_DISCLOSURE_CHAR_CAP,
        )
    }
}

// Trust-row disclosures need a smaller complete-text bound than dedicated command alerts.
pub(crate) const VERIFICATION_DISCLOSURE_CHAR_CAP: usize = 200;

/// Identify who may retry an unavailable verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryOwner {
    /// The user may press Retry.
    User,
    /// A load already owns this app's answer; no Retry is offered, because
    /// pressing it would race the load that is already running.
    PendingLoad,
    /// Nothing can be retried.
    Unretryable,
}

impl RetryOwner {
    /// Whether a Retry control may be rendered.
    #[must_use]
    pub(crate) fn offers_retry(self) -> bool {
        matches!(self, Self::User)
    }
}

/// The endpoint returns `{"verified": bool, "method": "...", ...}`.
#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code, reason = "Retain the deserialized upstream API wire shape")]
pub(crate) struct FlathubVerificationStatus {
    pub verified: bool,
    #[serde(default)]
    pub method: String,
}

/// Response from `GET /api/v2/stats/{app_id}`.
/// The endpoint returns `{"installs_total": n, "installs_last_month": n, ...}`.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[allow(dead_code, reason = "Retain the deserialized upstream API wire shape")]
pub(crate) struct FlathubAppStats {
    #[serde(default)]
    pub installs_total: u64,
    pub installs_last_month: u64,
    #[serde(default)]
    pub installs_last_7_days: u64,
}

/// Star-count histogram for a single app, as returned by the ODRS
/// (Open Desktop Ratings Service) `ratings` endpoint. `total` is the
/// number of reviews contributing to the histogram; `star1..star5`
/// each count how many reviews gave that star rating. `star0` in the
/// wire format is reserved (represents unrated — always zero in
/// practice) so we ignore it.
///
/// The wire format is `{"star1": n, ..., "star5": n, "total": n}`;
/// we accept `star0` too so serde doesn't reject a payload that
/// includes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(crate) struct RatingsCounts {
    #[serde(default)]
    pub star0: u64,
    #[serde(default)]
    pub star1: u64,
    #[serde(default)]
    pub star2: u64,
    #[serde(default)]
    pub star3: u64,
    #[serde(default)]
    pub star4: u64,
    #[serde(default)]
    pub star5: u64,
    #[serde(default)]
    pub total: u64,
}

impl RatingsCounts {
    /// Weighted-average star score across the histogram. Returns `0.0`
    /// when there are no reviews — caller decides whether to hide the
    /// row or show a "No reviews yet" fallback. Range on non-empty
    /// input: 1.0..=5.0.
    pub(crate) fn average(&self) -> f32 {
        let weighted =
            self.star1 + self.star2 * 2 + self.star3 * 3 + self.star4 * 4 + self.star5 * 5;
        let denom = self.star1 + self.star2 + self.star3 + self.star4 + self.star5;
        if denom == 0 {
            0.0
        } else {
            weighted as f32 / denom as f32
        }
    }

    /// Short human-readable form, e.g. `"4.5"`. Empty string when the
    /// app has no reviews — same signalling convention as the
    /// license / verified / age tiles.
    pub(crate) fn format_average(&self) -> String {
        if self.total == 0 {
            return String::new();
        }
        let avg = self.average();
        // Round to 1 decimal.
        let rounded = (avg * 10.0).round() / 10.0;
        // Strip trailing ".0" so a straight-5.0 reads as "5.0" (we keep
        // the tenths always to signal "this is a decimal average").
        format!("{rounded:.1}")
    }

    /// Total-reviews label, e.g. `"864 reviews"`. Empty when zero.
    pub(crate) fn format_total(&self) -> String {
        match self.total {
            0 => String::new(),
            1 => "1 review".to_owned(),
            n => format!("{n} reviews"),
        }
    }

    /// Star-count for a given bin, indexed 1..=5. Any other index
    /// returns 0 rather than panicking.
    pub(crate) fn bar(&self, star: u8) -> u64 {
        match star {
            1 => self.star1,
            2 => self.star2,
            3 => self.star3,
            4 => self.star4,
            5 => self.star5,
            _ => 0,
        }
    }

    /// Bar fill fraction (0.0..=1.0) for a given star bin, normalised
    /// against whichever bin has the most reviews. Used by the UI to
    /// scale the width of each row in the star-count histogram.
    /// Returns 0.0 when the app has no reviews.
    pub(crate) fn bar_fraction(&self, star: u8) -> f32 {
        let max = self
            .star1
            .max(self.star2)
            .max(self.star3)
            .max(self.star4)
            .max(self.star5);
        if max == 0 {
            return 0.0;
        }
        self.bar(star) as f32 / max as f32
    }
}

/// Look up a `RatingsCounts` for `app_id` in a fetched ODRS table.
/// Tries the raw id first (many Flathub app-ids are ODRS keys as-is)
/// and falls back to the short desktop-filename convention
/// (`<lowercased-last-segment>.desktop`). Returns `None` when the app
/// has no ODRS entry — the UI hides the ratings surface entirely.
///
/// Fallback rationale: ODRS pre-dates Flathub's tri-segment reverse-DNS
/// scheme and many apps only appear under their legacy `.desktop`
/// name. Firefox is the classic example: `org.mozilla.firefox` is
/// unknown to ODRS, but `firefox.desktop` has 809+ reviews.
pub(crate) fn lookup_rating<'a>(
    table: &'a std::collections::HashMap<String, RatingsCounts>,
    app_id: &str,
) -> Option<&'a RatingsCounts> {
    if let Some(v) = table.get(app_id) {
        return Some(v);
    }
    // Extract the last dot-separated segment and lowercase it.
    let last = app_id
        .rsplit('.')
        .next()
        .unwrap_or(app_id)
        .to_ascii_lowercase();
    if !last.is_empty() {
        let desktop_key = format!("{last}.desktop");
        if let Some(v) = table.get(&desktop_key) {
            return Some(v);
        }
    }
    None
}
