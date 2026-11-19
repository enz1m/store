//! Domain data and side-effect-free Store decisions.

use crate::services::AppEntry;

#[path = "model/browse.rs"]
mod browse;
#[path = "model/labels.rs"]
mod labels;
#[path = "model/operations.rs"]
mod operations;
#[path = "model/permissions.rs"]
mod permissions;
#[path = "model/ratings.rs"]
mod ratings;
#[path = "model/relevance.rs"]
mod relevance;
#[path = "model/remotes.rs"]
mod remotes;
pub(crate) use browse::{
    BROWSE_CATEGORIES, app_count_label, browse_category_icon, browse_category_label,
    task_count_label, update_count_label,
};
pub(crate) use labels::{library_total_size, see_all_changelogs_label};
pub(crate) use operations::{CancelState, OpKind, OperationId, OperationState, apply_progress};
pub(crate) use permissions::PermissionDefault;
pub(crate) use ratings::{
    content_rating_overall, format_age, verification_icon, verification_label,
};
pub(crate) use relevance::{format_installs, relevance_score};
pub(crate) use remotes::{AddRemoteForm, RemoteConflict, is_valid_remote_url, validate_remote};
#[cfg(test)]
pub(crate) use search::search_rows;
pub(crate) use search::{
    SearchFilters, category_rows_from, foss_filter_aria_label, hide_eol_filter_aria_label,
    installed_filter_aria_label, search_rows_from, verified_filter_aria_label,
};
pub(crate) use sorting::{
    CatalogSort, CollectionKind, LibrarySort, catalog_sorted_rows, library_rows,
};
#[path = "model/search.rs"]
mod search;
#[path = "model/sorting.rs"]
mod sorting;

#[cfg(test)]
#[cfg(not(test))]
pub(crate) use remotes::RemoteFormError;
