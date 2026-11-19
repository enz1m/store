//! Offline catalogue ingestion, indexing, persistence and startup regression tests.

use crate::catalog::{CACHE_FORMAT_ID, CACHE_VERSION, CatalogIndex, CatalogLoad};
use crate::{Message, Route, Store};
use enzui_core::persist::{InMemoryBackend, Store as PersistStore};

#[path = "catalog_tests/http_tests.rs"]
mod http_tests;

#[path = "catalog_tests/parse_tests.rs"]
mod parse_tests;

/// A `persist::Store` over an in-memory backend: a cache that cannot touch the
/// developer's home directory.
fn memory_cache() -> (PersistStore<CatalogIndex>, InMemoryBackend) {
    let backend = InMemoryBackend::new();
    let store = PersistStore::<CatalogIndex>::new(backend.clone(), CACHE_FORMAT_ID, CACHE_VERSION);
    (store, backend)
}

// Whether the rendered frame contains `needle` anywhere in its text or its
// accessible labels.
fn render_finds(store: &Store, needle: &str) -> bool {
    use iced_test::selector::Candidate;
    let needle = needle.to_owned();
    let view = store.view();
    let mut ui = iced_test::Simulator::new(view);
    ui.find(move |candidate: Candidate<'_>| match candidate {
        Candidate::Text { content, .. } if content.contains(&needle) => Some(()),
        Candidate::Custom { state, .. } => state
            .downcast_ref::<enzui_core::collect::SemanticsNode>()
            .and_then(|node| node.semantics.label.clone())
            .filter(|label| label.contains(&needle))
            .map(|_| ()),
        _ => None,
    })
    .is_ok()
}
