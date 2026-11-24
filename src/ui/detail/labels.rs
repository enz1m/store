//! Rating labels and dialog constants.

use super::*;

/// Reflect whether the ratings breakdown is visible while retaining its summary.
pub(crate) fn ratings_breakdown_toggle_label(open: bool) -> &'static str {
    if open {
        crate::i18n::label("Hide rating breakdown")
    } else {
        crate::i18n::label("Show rating breakdown")
    }
}

/// Summarize declared sandbox access from the same items the permissions dialog lists, broad access first.
pub(crate) fn permission_glance(perms: Option<&Permissions>) -> String {
    let Some(p) = perms else {
        return crate::i18n::label("Permission details are unavailable.").to_owned();
    };
    let mut items: Vec<PermissionItem> = permission_groups(p)
        .into_iter()
        .flat_map(|(_, items)| items)
        .collect();
    if items.is_empty() {
        return crate::i18n::label("Runs fully sandboxed \u{2014} no extra access requested.")
            .to_owned();
    }
    items.sort_by_key(|item| !item.broad);
    let titles: Vec<String> = items
        .iter()
        .take(GLANCE_TITLES)
        .map(|item| item.title.clone())
        .collect();
    let more = items.len() - titles.len();
    if more == 0 {
        crate::i18n::format(
            crate::i18n::localizer(),
            "Can access: {}.",
            &[("arg0", &(titles.join(", ")))],
        )
    } else {
        crate::i18n::format(
            crate::i18n::localizer(),
            "Can access: {} and {count} more.",
            &[("arg0", &(titles.join(", "))), ("count", &more)],
        )
    }
}

const GLANCE_TITLES: usize = 4;

// ---------------------------------------------------------------------------
// Single-source copy helpers — following the 1f57337 convention.
// ---------------------------------------------------------------------------

/// An unavailable detail context is loading, not a confirmed empty screenshot list.
pub(crate) fn loading_screenshots_copy() -> &'static str {
    crate::i18n::label("Loading screenshots\u{2026}")
}

pub(crate) fn whats_new_heading() -> &'static str {
    crate::i18n::label("What\u{2019}s New")
}

/// The ODRS attribution line rendered below the ratings histogram.
pub(crate) fn ratings_attribution() -> &'static str {
    crate::i18n::label("Ratings from Open Desktop Ratings Service (ODRS).")
}

/// The "Update all" button label in the Library view.
pub(crate) fn update_all_label() -> &'static str {
    crate::i18n::label("Update all")
}

/// The dimensionless maximum dialog-height fraction; multiply by window height rather than treating it as pixels.
pub(crate) const DIALOG_HEIGHT_FRACTION: f32 = 0.8;
