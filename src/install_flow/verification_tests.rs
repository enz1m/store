//! Publisher verification: verdicts, retries, icons, and trust admission.

use super::*;

#[test]
fn offline_home_and_detail_use_the_installed_apps_repository() {
    use crate::services::{AppEntry, ENZIMOS_REPOSITORY, InstallScope, RemoteInfo};

    let mut store = browse_store();
    store.remotes = vec![
        RemoteInfo {
            name: "enzimos".into(),
            url: ENZIMOS_REPOSITORY.url.into(),
            options: "system".into(),
            ..Default::default()
        },
        RemoteInfo {
            name: "enzimos".into(),
            url: "https://example.org/repo/".into(),
            options: "user".into(),
            ..Default::default()
        },
    ];
    let official = AppEntry {
        id: "org.test.Verified".into(),
        origin: "enzimos".into(),
        installed: true,
        install_scope: InstallScope::System,
        ..Default::default()
    };
    let other = AppEntry {
        install_scope: InstallScope::User,
        ..official.clone()
    };
    store.installed = vec![other.clone(), official.clone()];
    assert_eq!(store.enzimos_home_entries(), vec![&official]);
    store.enrich_installed();
    assert!(!store.installed[0].has_verified_badge());
    assert!(store.installed[1].has_verified_badge());

    let _ = store.update(Message::open_app_detail(&official));
    assert_eq!(label_count(&nodes(&store), "enzim Apps"), 1);
    assert_eq!(label_count(&nodes(&store), "Repository verified"), 1);
    assert!(!store.detail_metadata_authorized);

    let _ = store.update(Message::open_app_detail(&other));
    assert_eq!(label_count(&nodes(&store), "enzim Apps"), 0);
}

#[test]
fn search_promotes_verified_enzimos_apps_when_the_catalogue_arrives() {
    use crate::services::{ENZIMOS_REPOSITORY, RemoteInfo, RemoteLoad};

    let mut store = browse_store();
    let mut external = app("org.test.Calculator", "Calculator");
    external.verified = true;
    store.catalog.popular = vec![external, app("org.test.CalculatorPlus", "Calculator Plus")];
    store.route = Route::Search;
    store.search_popup_open = true;
    let _ = store.update(Message::SearchPopupInput("calculator".into()));
    assert_eq!(store.rows()[0].id, "org.test.Calculator");

    let official = AppEntry {
        from_enzimos: true,
        origin: ENZIMOS_REPOSITORY.origin.into(),
        ..app("org.enzimos.Calculator", "enzim Calculator")
    };
    let _ = store.update(Message::OwnedRemotesLoaded {
        request: store.remotes_read_request,
        result: Ok(vec![RemoteInfo {
            name: ENZIMOS_REPOSITORY.origin.into(),
            url: ENZIMOS_REPOSITORY.url.into(),
            options: "user".into(),
            ..Default::default()
        }]),
    });
    assert_eq!(store.remotes_load, RemoteLoad::Loaded);
    assert!(store.enzimos_apps.is_empty());
    assert!(
        store
            .repository_answer()
            .authorization(ENZIMOS_REPOSITORY.origin)
            .scope()
            .is_some()
    );

    let request = store.remotes_read_request;
    let _ = store.update(Message::EnzimosCatalogueLoaded {
        request: request.wrapping_sub(1),
        apps: Some(vec![official.clone()]),
    });
    assert!(store.enzimos_apps.is_empty());
    let _ = store.update(Message::EnzimosCatalogueLoaded {
        request,
        apps: Some(vec![official]),
    });
    assert_eq!(store.rows()[0].id, "org.enzimos.Calculator");
    assert_eq!(store.search_popup_results[0].id, "org.enzimos.Calculator");

    let _ = store.update(Message::ToggleVerifiedOnly);
    assert_eq!(
        store
            .rows()
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["org.enzimos.Calculator", "org.test.Calculator"],
    );
}

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

#[test]
fn deployed_flathub_metadata_follows_the_repository_url_in_its_own_installation() {
    use crate::services::{FLATHUB_REPOSITORY, InstallScope, RemoteInfo};

    let mut store = browse_store();
    let id = "org.test.Calculator";
    let mut listing = app(id, "Calculator");
    listing.description = "Public Flathub description".into();
    listing.icon_image = Some(crate::bounded_image::RenderImage::TrustedBundled(
        crate::bounded_image::TrustedBundledPath::new("shots/desktop-light.png"),
    ));
    store.catalog.popular = vec![listing];
    let flathub = AppEntry {
        origin: "community".into(),
        installed: true,
        install_scope: InstallScope::System,
        ..app(id, "Calculator")
    };
    let other = AppEntry {
        origin: FLATHUB_REPOSITORY.origin.into(),
        install_scope: InstallScope::User,
        ..flathub.clone()
    };
    store.installed = vec![other.clone(), flathub.clone()];
    store.remotes.clear();
    let _ = store.update(Message::open_app_detail(&flathub));
    assert!(!store.detail_metadata_authorized);

    let _ = store.update(Message::OwnedRemotesLoaded {
        request: store.remotes_read_request,
        result: Ok(vec![
            RemoteInfo {
                name: flathub.origin.clone(),
                url: FLATHUB_REPOSITORY.url.trim_end_matches('/').into(),
                options: "system".into(),
                ..Default::default()
            },
            RemoteInfo {
                name: other.origin.clone(),
                url: "https://example.org/repo/".into(),
                options: "user".into(),
                ..Default::default()
            },
        ]),
    });
    assert!(store.detail_metadata_authorized);
    let detailed = store.detail_entry(id).expect("system copy details");
    assert_eq!(detailed.origin, flathub.origin);
    assert_eq!(detailed.description, "Public Flathub description");
    assert!(detailed.icon_image.is_some());

    let _ = store.update(Message::open_app_detail(&other));
    assert!(!store.detail_metadata_authorized);
    let detailed = store.detail_entry(id).expect("user copy details");
    assert_eq!(detailed.origin, other.origin);
    assert!(detailed.description.is_empty());
    assert!(detailed.icon_image.is_none());
}
