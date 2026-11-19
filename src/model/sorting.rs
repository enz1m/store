//! Library and catalog sort orders.

use super::*;

// Library projections
// ---------------------------------------------------------------------------

/// How the Library list is ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum LibrarySort {
    #[default]
    Name,
    Size,
    Updates,
}

impl LibrarySort {
    pub(crate) fn label(self) -> &'static str {
        match self {
            LibrarySort::Name => crate::i18n::label("Name"),
            LibrarySort::Size => crate::i18n::label("Size"),
            LibrarySort::Updates => crate::i18n::label("Updates"),
        }
    }
}

/// Order + filter the installed list for the Library view.
///
/// `query` is matched case-insensitively against the name and the app id, so a
/// user who knows the reverse-DNS id can find the app without knowing its
/// display name.
///
/// `Size` sorts **descending** — the reason a user sorts a software library by
/// size is to find what to delete, and putting the 4 GB app last would defeat
/// that. `Updates` puts updatable apps first, then falls back to name so the
/// ordering is total and stable.
pub(crate) fn library_rows(
    installed: &[AppEntry],
    query: &str,
    sort: LibrarySort,
    updates: &[String],
) -> Vec<AppEntry> {
    let q = query.trim().to_ascii_lowercase();
    let matching = installed.iter().filter(|e| {
        q.is_empty()
            || e.name.to_ascii_lowercase().contains(&q)
            || e.id.to_ascii_lowercase().contains(&q)
    });

    // Deduplicate Library widget keys by app ID, prefer the actionable user copy, and retain other installations for disclosure; authorization reads the raw list.
    let mut rows: Vec<AppEntry> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for entry in matching {
        match index.get(&entry.id) {
            None => {
                index.insert(entry.id.clone(), rows.len());
                rows.push(entry.clone());
            }
            Some(&at) => {
                let base_is_user = rows[at].install_scope == crate::services::InstallScope::User;
                let new_is_user = entry.install_scope == crate::services::InstallScope::User;
                if new_is_user && !base_is_user {
                    // The user copy takes over as the actionable base; the row
                    // it displaces becomes a disclosed companion.
                    let displaced = std::mem::replace(&mut rows[at], entry.clone());
                    rows[at].also_installed_in = displaced.also_installed_in;
                    rows[at].also_installed_in.push(displaced.install_scope);
                } else {
                    rows[at].also_installed_in.push(entry.install_scope.clone());
                }
            }
        }
    }

    for row in &mut rows {
        row.installed = true;
        row.update_available = updates.iter().any(|u| u == &row.id);
    }

    match sort {
        LibrarySort::Name => rows.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        }),
        LibrarySort::Size => rows.sort_by(|a, b| {
            crate::services::parse_size_bytes(&b.size)
                .cmp(&crate::services::parse_size_bytes(&a.size))
                .then_with(|| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                })
        }),
        LibrarySort::Updates => rows.sort_by(|a, b| {
            b.update_available.cmp(&a.update_available).then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
        }),
    }
    rows
}

/// Identify an expanded Discover collection, sharing its heading and accessible name with the section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CollectionKind {
    Trending,
    Popular,
    RecentlyAdded,
    RecentlyUpdated,
}

impl CollectionKind {
    /// The section heading — the single source for both the rendered heading
    /// text and the `"See all {heading}"` accessible name.
    pub(crate) fn heading(self) -> &'static str {
        match self {
            CollectionKind::Trending => crate::i18n::label("Trending"),
            CollectionKind::Popular => crate::i18n::label("Popular"),
            CollectionKind::RecentlyAdded => crate::i18n::label("Recently Added"),
            CollectionKind::RecentlyUpdated => crate::i18n::label("Recently Updated"),
        }
    }

    /// The full (uncapped) collection of apps for this section.
    ///
    /// `Popular` re-sorts by download count descending through
    /// [`catalog_sorted_rows`], matching the section's own ordering.
    pub(crate) fn rows(
        self,
        catalog: &crate::services::FlathubCatalogs,
        _recently_uninstalled: &[AppEntry],
    ) -> Vec<AppEntry> {
        match self {
            CollectionKind::Trending => catalog.trending.clone(),
            CollectionKind::Popular => {
                catalog_sorted_rows(catalog, _recently_uninstalled, CatalogSort::Popularity)
            }
            CollectionKind::RecentlyAdded => catalog.new.clone(),
            CollectionKind::RecentlyUpdated => catalog.updated.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Discover catalogue sort orders (Cluster A)
// ---------------------------------------------------------------------------

/// Select a catalogue collection or the independent local uninstall history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum CatalogSort {
    /// Flathub's trending collection — the `TRENDING NOW` segment.
    #[default]
    Trending,
    /// Ordered by monthly download count, descending — the `Popularity` segment.
    Popularity,
}

/// Order each collection independently, breaking popularity ties by name; uninstall history is a separate source.
pub(crate) fn catalog_sorted_rows(
    catalog: &crate::services::FlathubCatalogs,
    _recently_uninstalled: &[AppEntry],
    sort: CatalogSort,
) -> Vec<AppEntry> {
    match sort {
        CatalogSort::Trending => catalog.trending.clone(),

        CatalogSort::Popularity => {
            let mut rows = catalog.popular.clone();
            rows.sort_by(|a, b| {
                b.downloads.cmp(&a.downloads).then_with(|| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                })
            });
            rows
        }
    }
}
