//! Cached catalog row projection shared by layout passes.
//!
//! The projection key includes every state input used to filter, sort, or
//! annotate rows; replacing a projection releases its previous rows.

use crate::model::{CatalogSort, LibrarySort, SearchFilters};
use crate::services::AppEntry;
use crate::{Route, Store};
use enzui_core::virtual_sequence::{ItemProvider, KeyedProvider, SequenceKey};
use std::hash::{Hash, Hasher};

/// Every input of the row projection, compared by value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProjectionKey {
    route: Route,
    library_query: String,
    library_sort: LibrarySort,
    search_query: String,
    filters: SearchFilters,
    browse_category: Option<String>,
    catalog_sort: CatalogSort,
    installed: u64,
    catalog: u64,
    /// A precomputed catalogue hash used on every update without hashing all entries again.
    catalog_index: u64,
    updates: u64,
    recently_uninstalled: u64,
}

// Hash a slice of entries **field by field**.
//
// The destructuring `let AppEntry { .. } = e` is deliberate and load-bearing:
// a new field on [`AppEntry`] breaks this function, which is the only
// mechanical guarantee that the invalidation key keeps covering everything a
// row can display.
pub(crate) fn hash_entries(entries: &[AppEntry]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    entries.len().hash(&mut h);
    for e in entries {
        let AppEntry {
            id,
            name,
            summary,
            origin,
            branch,
            version,
            size,
            downloads,
            installed,
            update_available,
            verified,
            from_enzimos,
            publisher,
            description,
            tags,
            icon_url,
            icon_image,
            license,
            is_eol,
            category,
            install_scope,
            also_installed_in,
            // Deployment identity admits the decoded icon, which is hashed below.
            deployed_arch: _,
            deployed_commit: _,
            removed_origin,
            enzimos_detail: _,
        } = e;
        id.hash(&mut h);
        name.hash(&mut h);
        summary.hash(&mut h);
        origin.hash(&mut h);
        branch.hash(&mut h);
        version.hash(&mut h);
        size.hash(&mut h);
        downloads.hash(&mut h);
        installed.hash(&mut h);
        update_available.hash(&mut h);
        verified.hash(&mut h);
        from_enzimos.hash(&mut h);
        publisher.hash(&mut h);
        description.hash(&mut h);
        tags.hash(&mut h);
        icon_url.hash(&mut h);
        icon_image.is_some().hash(&mut h);
        license.hash(&mut h);
        is_eol.hash(&mut h);
        category.hash(&mut h);
        // Scope changes alter row controls and must invalidate their projection.
        install_scope.hash(&mut h);
        // Other-installation disclosures are rendered state and belong in the cache key.
        also_installed_in.hash(&mut h);
        // A recorded source decides the recently-removed row's Reinstall control.
        removed_origin.hash(&mut h);
    }
    h.finish()
}

impl ProjectionKey {
    /// Read the key off the live store. Cheap relative to the projection it
    /// guards (hashing ~230 entries, no allocation per entry, no sort), and it
    /// runs once per **message**, not once per layout pass.
    pub(crate) fn of(store: &Store) -> Self {
        Self {
            route: store.route,
            library_query: store.library_query.clone(),
            library_sort: store.library_sort,
            search_query: store.search_query.clone(),
            filters: store.filters,
            browse_category: store.browse_category.clone(),
            catalog_sort: store.catalog_sort,
            installed: hash_entries(&store.installed),
            catalog: {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                hash_entries(&store.catalog.popular).hash(&mut h);
                hash_entries(&store.catalog.trending).hash(&mut h);
                hash_entries(&store.catalog.new).hash(&mut h);
                hash_entries(&store.catalog.updated).hash(&mut h);
                hash_entries(&store.enzimos_apps).hash(&mut h);
                h.finish()
            },
            catalog_index: store.catalog_index.content_hash(),
            updates: {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                store.update_discovery.hash(&mut h);
                h.finish()
            },
            recently_uninstalled: hash_entries(&store.recently_uninstalled),
        }
    }
}

/// The rows one route shows, plus the key they were computed from.
///
/// Implements [`KeyedProvider`] over its own rows so it can *be* the virtual
/// list's provider: the borrow the list needs is a borrow of the `Store`.
#[derive(Debug, Default)]
pub(crate) struct RowProjection {
    key: Option<ProjectionKey>,
    rows: Vec<AppEntry>,
    /// Discover's popularity-ordered collection (`model::catalog_sorted_rows`),
    /// which used to be re-sorted on every build in `ui::discover`.
    popular: Vec<AppEntry>,
}

impl RowProjection {
    /// The projection for `store`'s current route.
    pub(crate) fn compute(store: &Store) -> Self {
        let rows = match store.route {
            Route::Library => crate::model::library_rows(
                &store.installed,
                &store.library_query,
                store.library_sort,
                // The row set tracks app identities; the reported scope determines the control.
                &store.update_ids(),
            ),
            // Borrow the complete catalogue and clone only matching rows.
            Route::Search => crate::model::search_rows_from(
                store.catalog_candidates().into_iter(),
                &store.search_query,
                store.filters,
                &store.installed_ids(),
                &store.update_ids(),
            ),
            Route::Categories => match &store.browse_category {
                Some(key) => {
                    let mut r = crate::model::category_rows_from(
                        store.catalog_candidates().into_iter(),
                        key.as_str(),
                    );
                    let installed = store.installed_ids();
                    for row in &mut r {
                        row.installed = installed.contains(&row.id);
                    }
                    r
                }
                None => Vec::new(),
            },
            Route::Detail | Route::Discover => Vec::new(),
        };
        let popular = match store.route {
            Route::Discover => crate::model::catalog_sorted_rows(
                &store.catalog,
                &store.recently_uninstalled,
                CatalogSort::Popularity,
            ),
            _ => Vec::new(),
        };
        Self {
            key: Some(ProjectionKey::of(store)),
            rows,
            popular,
        }
    }

    /// A projection over rows a caller supplied directly (the direct-`view`
    /// render tests). Carries no key: it is never served to the routed view.
    pub(crate) fn adopted(rows: Vec<AppEntry>) -> Self {
        Self {
            key: None,
            rows,
            popular: Vec::new(),
        }
    }

    pub(crate) fn rows(&self) -> &[AppEntry] {
        &self.rows
    }

    pub(crate) fn popular(&self) -> &[AppEntry] {
        &self.popular
    }

    /// Whether this projection was computed from `key`.
    pub(crate) fn matches(&self, key: &ProjectionKey) -> bool {
        self.key.as_ref() == Some(key)
    }
}

impl ItemProvider<AppEntry> for RowProjection {
    fn len(&self) -> u64 {
        self.rows.len() as u64
    }

    fn item(&self, index: u64) -> Option<AppEntry> {
        self.rows.get(usize::try_from(index).ok()?).cloned()
    }
}

impl KeyedProvider<AppEntry> for RowProjection {
    fn key(&self, index: u64) -> Option<SequenceKey> {
        self.rows
            .get(usize::try_from(index).ok()?)
            .map(|e| SequenceKey::new(e.id.clone()))
    }
}

// ---------------------------------------------------------------------------
// The test-only staleness check, and the one place it is turned off
// ---------------------------------------------------------------------------

#[cfg(test)]
thread_local! {
    static STALENESS_CHECKS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether `Store::projection` re-derives the key to prove a filled cache is
/// current. On in every test by default; the shipped binary has no such check
/// at all (it is `cfg(test)`), which is why the timing test turns it off — a
/// measurement of code the user never runs is not a measurement.
#[cfg(test)]
pub(crate) fn staleness_checks_enabled() -> bool {
    STALENESS_CHECKS.with(std::cell::Cell::get)
}
