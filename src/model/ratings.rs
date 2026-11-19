//! Content rating, verification, and age formatting.

// Content rating and verification (Cluster C)
// ---------------------------------------------------------------------------

/// Format the content-rating "Overall" line.
///
/// `"Overall: "` is a *prefix* with a value appended. The composed string
/// is asserted exactly once.
pub(crate) fn content_rating_overall(text: &str) -> String {
    crate::i18n::format(
        crate::i18n::localizer(),
        "Overall: {text}",
        &[("text", &(text))],
    )
}

/// Derive a trust label from the verdict, preserving unknown as distinct from unverified.
#[must_use]
pub(crate) fn verification_label(
    verification: &crate::services::PublisherVerification,
) -> &'static str {
    use crate::services::PublisherVerification as V;
    match verification {
        V::Verified { .. } => crate::i18n::label("Publisher verified"),
        V::KnownUnverified { .. } => crate::i18n::label("Not verified"),
        // An absent answer cannot establish a negative verification verdict.
        V::Unknown(_) => crate::i18n::label("Verification unavailable"),
    }
}

/// Use a distinct registered icon for each verification state, including unknown.
#[must_use]
pub(crate) fn verification_icon(
    verification: &crate::services::PublisherVerification,
) -> &'static str {
    use crate::services::PublisherVerification as V;
    match verification {
        V::Verified { .. } => "badge-check",
        V::KnownUnverified { .. } => "shield-alert",
        V::Unknown(_) => "circle-question-mark",
    }
}

/// Round retained-answer age down to a coarse unit so it never appears younger than it is.
#[must_use]
pub(crate) fn format_age(age_ms: u64) -> String {
    const MINUTE: u64 = 60 * 1000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    match age_ms {
        ms if ms < MINUTE => crate::i18n::label("less than a minute").to_owned(),
        ms if ms < HOUR => crate::i18n::quantity("app-age-minutes", ms / MINUTE),
        ms if ms < DAY => crate::i18n::quantity("app-age-hours", ms / HOUR),
        ms => crate::i18n::quantity("app-age-days", ms / DAY),
    }
}
