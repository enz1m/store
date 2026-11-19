//! Parse boundary and hostile payloads.

use super::http_tests::{
    HTTP_SUCCESS_APP_NAME, assert_dump_has_no_parser_detail, count_runs_containing,
    discover_after_load, hostile_invalid_document, render_text_dump, rendered_text_runs,
    successful_http_source,
};
use super::*;
use crate::catalog::HttpCatalogSource;
use crate::services::INVALID_DOCUMENT_SUMMARY;

// Warm-cache stale fallback stays safe when a later refresh returns a hostile
// malformed payload: the saved catalogue survives, one stale surface, no leak.
#[test]
fn hostile_parse_over_warm_cache_uses_one_stale_surface_via_reducer() {
    let (cache, _backend) = memory_cache();
    let good = successful_http_source();
    let warm = crate::load_catalog_cached(&good, 1_000, Some(&cache), None, false);
    assert_eq!(warm.index.len(), 1);
    assert!(!warm.stale);

    let hostile = HttpCatalogSource::with_transport(|_| Ok(hostile_invalid_document()));
    let later = 1_000 + crate::catalog::CATALOG_TTL_SECS + 1;
    let load = crate::load_catalog_cached(&hostile, later, Some(&cache), None, false);
    assert_eq!(load.index.len(), 1, "the saved catalogue must survive");
    assert!(load.stale);
    assert_eq!(load.error.as_deref(), Some(INVALID_DOCUMENT_SUMMARY));

    let store = discover_after_load(load);
    let dump = render_text_dump(&store);
    let runs = rendered_text_runs(&store);
    assert_eq!(
        count_runs_containing(&dump, crate::ui::discover::stale_banner_title()),
        1,
        "the stale surface must be surfaced exactly once: {dump:?}"
    );
    assert_eq!(
        count_runs_containing(&runs, crate::ui::discover::RETRY_LABEL),
        1,
        "the stale surface must offer one retry control: {runs:?}"
    );
    assert!(render_finds(&store, HTTP_SUCCESS_APP_NAME));
    assert!(!render_finds(
        &store,
        crate::ui::discover::catalog_error_title()
    ));
    assert!(!render_finds(&store, INVALID_DOCUMENT_SUMMARY));
    assert_dump_has_no_parser_detail(&dump, "warm-cache-parse");
}
