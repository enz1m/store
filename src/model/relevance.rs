//! Search ranking and install-count formatting.

use super::*;

// ---------------------------------------------------------------------------
// Search ranking  (ported from round one)
// ---------------------------------------------------------------------------

// Relevance score (higher = better match):
//   100  exact name match (case-insensitive)
//    80  name starts with query
//    60  name contains query
//    40  summary contains query
//    20  description / tags contain query
//     0  no match
//
// Boosts (added on top):
//   +5  verified publisher
//   +3  popular (>10K downloads/mo)

/// Relevance score for a search result. Higher = better match. 0 = no match.
///
/// `query` must already be lowercased and trimmed by the caller.
pub(crate) fn relevance_score(entry: &AppEntry, query: &str) -> u32 {
    if query.is_empty() {
        // Empty query: everything matches with a base score so the caller can
        // apply boosts and sort deterministically.
        return 50 + verified_boost(entry) + popularity_boost(entry);
    }
    let name_lower = entry.name.to_ascii_lowercase();

    let base = if name_lower == query {
        100
    } else if name_lower.starts_with(query) {
        80
    } else if name_lower.contains(query) {
        60
    } else if entry.summary.to_ascii_lowercase().contains(query) {
        40
    } else if entry.description.to_ascii_lowercase().contains(query)
        || entry
            .tags
            .iter()
            .any(|t| t.to_ascii_lowercase().contains(query))
        // Include reverse-DNS and binary-name matches even when display text omits them.
        || entry.id.to_ascii_lowercase().contains(query)
    {
        20
    } else {
        return 0; // No match — no boosts
    };

    base + verified_boost(entry) + popularity_boost(entry)
}

fn verified_boost(entry: &AppEntry) -> u32 {
    if entry.verified { 5 } else { 0 }
}

fn popularity_boost(entry: &AppEntry) -> u32 {
    if entry.downloads > 10_000 { 3 } else { 0 }
}

// ---------------------------------------------------------------------------
// Install-count formatting  (ported from round one)
// ---------------------------------------------------------------------------

/// Human-readable install count. `0` → `""` so the caller can hide the row.
///
/// Chosen over showing a raw integer because a store's popularity signal is
/// approximate by nature; "824,371 installs" reads as false precision.
pub(crate) fn format_installs(downloads: u64) -> String {
    if downloads == 0 {
        return String::new();
    }
    if downloads < 1_000 {
        return crate::i18n::format(
            crate::i18n::localizer(),
            "{downloads} installs",
            &[("downloads", &downloads)],
        );
    }
    let (divisor, source) = if downloads < 1_000_000 {
        (1_000_u64, "{count}K installs")
    } else {
        (1_000_000_u64, "{count}M installs")
    };
    let tenths = downloads.saturating_add(divisor / 20) / (divisor / 10);
    let count = tenths as f64 / 10.0;
    crate::i18n::format(crate::i18n::localizer(), source, &[("count", &count)])
}
