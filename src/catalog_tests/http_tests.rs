//! HTTP catalogue pages and warm-cache failures.

use super::*;

// ---------------------------------------------------------------------------
// The production HTTP producer translates transport failures to bounded safe
// copy before the ingest boundary, retaining raw diagnostics for escaped logs.
// ---------------------------------------------------------------------------

use crate::catalog::HttpCatalogSource;

use crate::services::{HttpFetchError, NETWORK_SUMMARY};

pub(super) const HTTP_SUCCESS_APP_NAME: &str = "HTTP Seam Success";

fn valid_http_collection() -> String {
    r#"{
        "hits": [{
            "app_id": "org.test.HttpSuccess",
            "name": "HTTP Seam Success",
            "summary": "Loaded through the HTTP catalogue producer",
            "main_categories": "network"
        }],
        "page": 1,
        "totalPages": 1,
        "totalHits": 1
    }"#
    .to_owned()
}

pub(super) fn successful_http_source() -> HttpCatalogSource {
    HttpCatalogSource::with_transport(|_| Ok(valid_http_collection()))
}

/// Collect every rendered text run and accessible label the frame publishes.
pub(super) fn render_text_dump(store: &Store) -> Vec<String> {
    use iced_test::selector::Candidate;
    let view = store.view();
    let mut ui = iced_test::Simulator::new(view);
    let mut dump: Vec<String> = Vec::new();
    let _ = ui.find(|candidate: Candidate<'_>| -> Option<()> {
        match candidate {
            Candidate::Text { content, .. } => dump.push(content.to_owned()),
            Candidate::Custom { state, .. } => {
                if let Some(node) = state.downcast_ref::<enzui_core::collect::SemanticsNode>()
                    && let Some(label) = node.semantics.label.clone()
                {
                    dump.push(label);
                }
            }
            _ => {}
        }
        None
    });
    dump
}

/// Collect visible text runs without also counting their accessible container
/// labels. This is used to prove that one status surface is painted exactly
/// once rather than merely reachable somewhere in the semantics tree.
pub(super) fn rendered_text_runs(store: &Store) -> Vec<String> {
    use iced_test::selector::Candidate;
    let view = store.view();
    let mut ui = iced_test::Simulator::new(view);
    let mut runs = Vec::new();
    let _ = ui.find(|candidate: Candidate<'_>| -> Option<()> {
        if let Candidate::Text { content, .. } = candidate {
            runs.push(content.to_owned());
        }
        None
    });
    runs
}

pub(super) fn count_runs_containing(runs: &[String], needle: &str) -> usize {
    runs.iter().filter(|text| text.contains(needle)).count()
}

/// No rendered candidate may carry a control character, ANSI/OSC payload,
/// errno tail, Debug argv, or the raw hostile body.
pub(super) fn assert_dump_is_safe(dump: &[String], context: &str) {
    for text in dump {
        assert!(
            !text.chars().any(char::is_control),
            "{context}: rendered text carries a control character: {text:?}"
        );
        assert!(
            !text.contains("31m") && !text.contains("0;secret"),
            "{context}: rendered text carries terminal-sequence payload: {text:?}"
        );
        assert!(
            !text.contains("(os error"),
            "{context}: rendered text leaks an errno tail: {text:?}"
        );
        assert!(
            !text.contains("[\""),
            "{context}: rendered text leaks a Debug argv: {text:?}"
        );
    }
}

/// Build a settled Discover store, then run the load through the real reducer.
pub(super) fn discover_after_load(load: CatalogLoad) -> Store {
    let (mut store, _) = Store::boot_settled(false);
    store.loading_installed = false;
    store.loading_catalog = false;
    store.route = Route::Discover;
    let _ = store.update(Message::CatalogLoaded(Box::new(load)));
    store
}

pub(super) fn assert_only_success_surface(store: &Store, context: &str) {
    assert!(
        render_finds(store, HTTP_SUCCESS_APP_NAME),
        "{context}: producer success must render its app"
    );
    assert!(!render_finds(
        store,
        crate::ui::discover::catalog_error_title()
    ));
    assert!(!render_finds(
        store,
        crate::ui::discover::stale_banner_title()
    ));
    assert!(!render_finds(store, "No apps available yet."));
    assert!(!render_finds(store, "Loading Flathub catalog\u{2026}"));
}

#[test]
fn http_success_runs_through_producer_reducer_and_discover_controls() {
    let source = successful_http_source();
    let load = crate::refresh_catalog_with_source(&source, 1_000, None, None, false, &|| false);
    assert_eq!(load.requests, 4, "one page and three fetched rails");
    assert!(!load.stale);
    assert!(load.error.is_none());
    assert_eq!(load.index.len(), 1);

    let store = discover_after_load(load);
    assert_only_success_surface(&store, "HTTP success");
    assert_dump_is_safe(&render_text_dump(&store), "HTTP success");
}

#[test]
fn transport_failure_over_warm_http_cache_uses_one_stale_surface_via_reducer() {
    let (cache, _backend) = memory_cache();
    let good = successful_http_source();
    let warm = crate::load_catalog_cached(&good, 1_000, Some(&cache), None, false);
    assert_eq!(warm.requests, 4);
    assert_eq!(warm.index.len(), 1);
    assert!(!warm.stale);

    let failed = HttpCatalogSource::with_transport(|_| {
        Err(HttpFetchError::Transport {
            code: "backend",
            backend_code: Some(6),
        })
    });
    let later = 1_000 + crate::catalog::CATALOG_TTL_SECS + 1;
    let load = crate::load_catalog_cached(&failed, later, Some(&cache), None, false);
    assert_eq!(load.index.len(), 1, "the saved catalogue must survive");
    assert!(load.stale);
    assert_eq!(load.error.as_deref(), Some(NETWORK_SUMMARY));

    let store = discover_after_load(load);
    // The stale banner's title is an accessible label, so it is counted over
    // the label-inclusive dump; its retry control is a real button `Text` run.
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
    assert!(!render_finds(&store, NETWORK_SUMMARY));
    assert!(!render_finds(&store, "No apps available yet."));
    assert!(!render_finds(&store, "Loading Flathub catalog\u{2026}"));
    assert_dump_is_safe(&render_text_dump(&store), "warm-cache");
}

// ---------------------------------------------------------------------------
// Catalogue *payload* (JSON parse) failures are classified into fixed bounded
// categories at the parser boundary. The raw `serde_json` `Display` (parser
// vocabulary, line/column, and echoed attacker snippets) reaches only the
// escaped log, never Discover.
// ---------------------------------------------------------------------------

/// A marker string an attacker plants in the payload; it must never reach a UI.
pub(super) const PARSE_MARKER: &str = "PARSEMARKER";
/// A long non-ASCII tail a naive interpolation of the payload would spill.
pub(super) const PARSE_TAIL_MARK: char = '\u{5b57}';

/// Structurally invalid JSON: an unterminated collection literal carrying a
/// bidi override, an ANSI run, a NUL, the marker, and a long non-ASCII tail —
/// the worst a syntax fault could spill through `serde_json`'s `Display`.
pub(super) fn hostile_invalid_document() -> String {
    format!(
        "{{\"hits\": [ \u{202e}\u{1b}[31m {PARSE_MARKER} \u{0} {}",
        PARSE_TAIL_MARK.to_string().repeat(500)
    )
}

/// The rendered surface must carry none of the parser's raw detail: no marker,
/// no parser vocabulary, no line/column coordinate, no control/bidi byte.
pub(super) fn assert_dump_has_no_parser_detail(dump: &[String], context: &str) {
    assert_dump_is_safe(dump, context);
    for text in dump {
        assert!(
            !text.contains(PARSE_MARKER),
            "{context}: rendered text carries the attacker payload marker: {text:?}"
        );
        assert!(
            !text.contains(PARSE_TAIL_MARK),
            "{context}: rendered text carries the raw hostile tail: {text:?}"
        );
        // Distinctive `serde_json` `Display` phrases that could only appear if
        // a raw parser diagnostic reached the surface. Kept narrow so ordinary
        // English UI copy cannot trip the guard.
        for token in [
            "invalid type",
            "at line",
            "control character",
            "a sequence",
            "EOF while",
            "trailing characters",
        ] {
            assert!(
                !text.contains(token),
                "{context}: rendered text leaks parser vocabulary {token:?}: {text:?}"
            );
        }
    }
}
