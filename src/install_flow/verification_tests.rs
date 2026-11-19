//! Publisher verification: verdicts, retries, icons, and trust admission.

use super::*;

#[test]
fn a_stale_detail_context_cannot_publish_its_trust_state_onto_another_app() {
    let mut store = browse_store();
    let _ = store.update(Message::OpenAppDetail("org.test.Verified".into()));
    let stale_generation = store.detail_generation;
    let stale_cancellation = store.detail_cancellation.clone();

    // The user moves on before the first enrichment lands.
    let _ = store.update(Message::OpenAppDetail("org.test.Hero".into()));
    assert!(
        stale_cancellation.is_cancelled(),
        "selecting B must cancel A's transports before they can publish"
    );
    assert!(
        store.detail_context().is_none(),
        "a fresh page starts with no context"
    );

    // App A's enrichment arrives late, carrying a verified publisher.
    let _ = store.update(Message::DetailContextLoaded {
        app_id: "org.test.Verified".into(),
        generation: stale_generation,
        context: Box::new(crate::services::DetailContext {
            verification: live_verified(),
            ..Default::default()
        }),
    });
    assert!(
        store.detail_context().is_none(),
        "it is discarded outright — not merged, not partially applied"
    );
    let ns = nodes(&store);
    assert_eq!(
        label_count(&ns, "Publisher verified"),
        0,
        "app B must not wear app A's badge; labels: {:?}",
        labels(&ns)
    );
    assert_eq!(
        label_count(&ns, "Checking Flathub for publisher verification."),
        1,
        "B's own check is still running, and its placeholder says so; labels: {:?}",
        labels(&ns)
    );
    assert_eq!(
        label_count(&ns, "Verification unavailable"),
        0,
        "a check still running is not shown as a failure; labels: {:?}",
        labels(&ns)
    );

    // Returning to A creates a replacement generation. The first A terminal
    // still cannot publish merely because the selected id matches again.
    let _ = store.update(Message::OpenAppDetail("org.test.Verified".into()));
    let replacement_generation = store.detail_generation;
    assert_ne!(replacement_generation, stale_generation);
    let _ = store.update(Message::DetailContextLoaded {
        app_id: "org.test.Verified".into(),
        generation: stale_generation,
        context: Box::new(crate::services::DetailContext {
            verification: live_verified(),
            ..Default::default()
        }),
    });
    assert!(
        store.detail_context().is_none(),
        "the first A visit cannot publish into the replacement A generation"
    );

    // A context whose own fetch was cancelled is refused even when the ids
    // and generation match, because the ids match again when the user returns.
    let _ = store.update(Message::DetailContextLoaded {
        app_id: "org.test.Verified".into(),
        generation: store.detail_generation,
        context: Box::new(crate::services::DetailContext {
            verification: live_verified(),
            cancelled: true,
            ..Default::default()
        }),
    });
    assert!(
        store.detail_context().is_none(),
        "an abandoned fetch is not an answer"
    );
}

#[test]
fn an_unknown_verification_is_never_admitted_by_a_verified_only_filter() {
    use crate::services::{PublisherVerification, VerificationFailure};

    for reason in [
        VerificationFailure::Pending,
        VerificationFailure::Cancelled,
        VerificationFailure::Policy,
        VerificationFailure::Transport("down".into()),
        VerificationFailure::Status("HTTP 500".into()),
        VerificationFailure::Schema("bad".into()),
        VerificationFailure::NotRequested,
    ] {
        assert!(
            !PublisherVerification::unavailable(reason.clone()).is_verified(),
            "{reason:?} must not answer a verified-only question affirmatively"
        );
    }

    // And the filter itself excludes a row with no verification claim, which
    // is what an unenriched catalogue row is.
    let unclaimed = app("org.test.Unclaimed", "Unclaimed");
    assert!(!unclaimed.verified, "the fixture row makes no claim");
    let filters = crate::model::SearchFilters {
        verified_only: true,
        ..Default::default()
    };
    let rows = crate::model::search_rows(
        std::slice::from_ref(&unclaimed),
        "Unclaimed",
        filters,
        &[],
        &[],
    );
    assert!(
        rows.is_empty(),
        "a row with no verification claim is excluded, not admitted"
    );

    // Positive control: a claimed row IS admitted, so the clause above is
    // attributable to the claim and not to a broken query.
    let mut claimed = app("org.test.Claimed", "Claimed");
    claimed.verified = true;
    let rows =
        crate::model::search_rows(std::slice::from_ref(&claimed), "Claimed", filters, &[], &[]);
    assert_eq!(rows.len(), 1);
}

// A failed update read remains unknown rather than implying zero updates.
