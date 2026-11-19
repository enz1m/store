//! Release history and category labels.

use super::*;

/// The button label for opening the complete changelog history.
pub(crate) fn see_all_changelogs_label() -> &'static str {
    crate::i18n::label("See all Changelogs")
}

/// Total on-disk size of the installed set, formatted.
pub(crate) fn library_total_size(installed: &[AppEntry]) -> String {
    let total: u64 = installed
        .iter()
        .map(|e| crate::services::parse_size_bytes(&e.size))
        .sum();
    crate::services::format_size_bytes(total)
}
