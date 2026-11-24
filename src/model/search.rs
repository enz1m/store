//! Search filters and row projections.

use super::*;

// Search projection
// ---------------------------------------------------------------------------

/// Filters the Search view applies on top of the relevance ranking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SearchFilters {
    pub verified_only: bool,
    pub foss_only: bool,
    pub hide_eol: bool,
    pub installed_only: bool,
}

impl SearchFilters {
    pub(crate) fn any(self) -> bool {
        self.verified_only || self.foss_only || self.hide_eol || self.installed_only
    }
}

/// Is the SPDX expression a free/open-source licence?
///
/// Deliberately conservative: an **empty** licence string is unknown, not
/// proprietary, and "FOSS only" must not silently hide every app whose
/// appstream fetch has not landed yet. Unknown passes the filter; only an
/// explicit `LicenseRef-proprietary` / `Proprietary` is excluded.
pub(crate) fn is_foss(license: &str) -> bool {
    let l = license.trim().to_ascii_lowercase();
    if l.is_empty() {
        return true;
    }
    !(l.contains("proprietary") || l.contains("licenseref-proprietary"))
}

/// Rank and deduplicate test-owned candidates by app ID, retaining the highest-scoring copy.
#[cfg(test)]
pub(crate) fn search_rows(
    candidates: &[AppEntry],
    query: &str,
    filters: SearchFilters,
    installed_ids: &[String],
    updates: &[String],
) -> Vec<AppEntry> {
    search_rows_from(candidates.iter(), query, filters, installed_ids, updates)
}

/// Search borrowed catalogue entries and clone only matching results.
pub(crate) fn search_rows_from<'a>(
    candidates: impl Iterator<Item = &'a AppEntry>,
    query: &str,
    filters: SearchFilters,
    installed_ids: &[String],
    updates: &[String],
) -> Vec<AppEntry> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }

    let mut best = Vec::<((bool, u32), AppEntry)>::new();
    for entry in candidates {
        let score = relevance_score(entry, &q);
        if score == 0 {
            continue;
        }
        let mut entry = entry.clone();
        entry.installed = installed_ids.iter().any(|i| i == &entry.id);
        entry.update_available = updates.iter().any(|u| u == &entry.id);

        if filters.verified_only && !entry.has_verified_badge() {
            continue;
        }
        if filters.foss_only && !is_foss(&entry.license) {
            continue;
        }
        if filters.hide_eol && entry.is_eol {
            continue;
        }
        if filters.installed_only && !entry.installed {
            continue;
        }

        let rank = (entry.from_enzimos, score);
        match best.iter_mut().find(|(_, e)| e.id == entry.id) {
            Some(slot) if slot.0 < rank => *slot = (rank, entry),
            Some(_) => {}
            None => best.push((rank, entry)),
        }
    }

    best.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| {
            a.1.name
                .to_ascii_lowercase()
                .cmp(&b.1.name.to_ascii_lowercase())
        })
    });
    best.into_iter().map(|(_, e)| e).collect()
}

/// Filter borrowed catalogue entries without cloning the complete source.
pub(crate) fn category_rows_from<'a>(
    catalog: impl Iterator<Item = &'a AppEntry>,
    category: &str,
) -> Vec<AppEntry> {
    let key = category.trim().to_ascii_lowercase();
    let mut rows: Vec<AppEntry> = catalog
        .filter(|e| e.category.eq_ignore_ascii_case(&key))
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        b.downloads.cmp(&a.downloads).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    rows.dedup_by(|a, b| a.id == b.id);
    rows
}

// ---------------------------------------------------------------------------
// FOSS filter accessible labels (Cluster B)
// ---------------------------------------------------------------------------

/// Announce the FOSS filter state and the action its next press performs.
pub(crate) fn foss_filter_aria_label(active: bool) -> &'static str {
    if active {
        crate::i18n::label("FOSS-only filter on. Press to turn off.")
    } else {
        crate::i18n::label("FOSS-only filter off. Press to turn on.")
    }
}

/// Announce the Verified toggle's current state and the action its next press performs.
pub(crate) fn verified_filter_aria_label(active: bool) -> &'static str {
    if active {
        crate::i18n::label("Verified filter on. Press to turn off.")
    } else {
        crate::i18n::label("Verified filter off. Press to turn on.")
    }
}

/// Announce the end-of-life filter state and the action its next press performs.
pub(crate) fn hide_eol_filter_aria_label(active: bool) -> &'static str {
    if active {
        crate::i18n::label("Hide end-of-life filter on. Press to turn off.")
    } else {
        crate::i18n::label("Hide end-of-life filter off. Press to turn on.")
    }
}

/// Returns the Installed-only filter's current state and next action.
pub(crate) fn installed_filter_aria_label(active: bool) -> &'static str {
    if active {
        crate::i18n::label("Installed-only filter on. Press to turn off.")
    } else {
        crate::i18n::label("Installed-only filter off. Press to turn on.")
    }
}
