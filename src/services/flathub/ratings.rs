fn fetch_all_ratings_with_http(
    fixture: bool,
    http: &dyn MetadataHttp,
    cancellation: &OperationCancellation,
) -> Result<std::collections::HashMap<String, RatingsCounts>, String> {
    if fixture {
        return Ok(mock_catalog()
            .into_iter()
            .map(|entry| (entry.id, fixture_ratings()))
            .collect());
    }
    let raw = http
        .get(
            "https://odrs.gnome.org/1.0/reviews/api/ratings",
            MetadataRoute::Ratings,
            cancellation,
        )
        .map_err(|error| error.to_string())?;
    parse_ratings_table(&raw)
}

/// Parse the ODRS ratings-table JSON. Split from [`fetch_all_ratings`]
/// so tests can drive it with a fixture.
pub(crate) fn parse_ratings_table(
    raw: &str,
) -> Result<std::collections::HashMap<String, RatingsCounts>, String> {
    let table: std::collections::HashMap<String, RatingsCounts> =
        serde_json::from_str(raw).map_err(|e| format!("JSON parse: {e}"))?;
    if table.len() > MAX_RATINGS_RECORDS {
        return Err("ratings table exceeds its decoded record limit".to_owned());
    }
    let mut aggregate = 0u64;
    for (key, counts) in &table {
        if key.len() > 512 {
            return Err("ratings key exceeds its decoded string limit".to_owned());
        }
        aggregate = aggregate
            .checked_add(validate_rating_counts(counts)?)
            .ok_or_else(|| "ratings aggregate overflow".to_owned())?;
        if aggregate > MAX_RATINGS_AGGREGATE {
            return Err("ratings table exceeds its decoded aggregate limit".to_owned());
        }
    }
    Ok(table)
}

fn validate_rating_counts(counts: &RatingsCounts) -> Result<u64, String> {
    let mut aggregate = 0u64;
    for count in [
        counts.star0,
        counts.star1,
        counts.star2,
        counts.star3,
        counts.star4,
        counts.star5,
        counts.total,
    ] {
        if count > MAX_RATING_COUNT {
            return Err("ratings row exceeds its decoded count limit".to_owned());
        }
        aggregate = aggregate
            .checked_add(count)
            .ok_or_else(|| "ratings aggregate overflow".to_owned())?;
    }
    Ok(aggregate)
}

const DETAIL_RATINGS_CACHE_SCHEMA: u32 = 2;
const DETAIL_RATINGS_CACHE_MAX_BYTES: usize = 4096;
const RATINGS_ENDPOINT: &str = "https://odrs.gnome.org/1.0/reviews/api/ratings";

#[derive(serde::Serialize, serde::Deserialize)]
struct DetailRatingsCacheEnvelope {
    schema: u32,
    app_id: String,
    #[serde(alias = "origin")]
    repository: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    scope: String,
    route: String,
    fetched_at: u64,
    rating: Option<RatingsCounts>,
}

fn detail_ratings_cache_name(app_id: &str) -> String {
    let repository = FLATHUB_REPOSITORY.url;
    let identity = format!(
        "schema={DETAIL_RATINGS_CACHE_SCHEMA}\nrepository={repository}\nroute={RATINGS_ENDPOINT}\napp={app_id}"
    );
    format!(
        "detail-ratings-v{DETAIL_RATINGS_CACHE_SCHEMA}-{}.json",
        cache_identity(CacheIdentityDomain::HostId, identity.as_bytes())
    )
}

fn legacy_flathub_ratings_name(app_id: &str, scope: &str) -> String {
    let origin = FLATHUB_REPOSITORY.origin;
    let identity = format!("schema=1\norigin={origin}\nscope={scope}\nroute={RATINGS_ENDPOINT}\napp={app_id}");
    format!(
        "detail-ratings-v1-{}.json",
        cache_identity(CacheIdentityDomain::HostId, identity.as_bytes())
    )
}

fn detail_ratings_payload(
    app_id: &str,
    fetched_at: u64,
    rating: Option<&RatingsCounts>,
) -> Vec<u8> {
    serde_json::to_vec(&DetailRatingsCacheEnvelope {
        schema: DETAIL_RATINGS_CACHE_SCHEMA,
        app_id: app_id.to_owned(),
        repository: FLATHUB_REPOSITORY.url.to_owned(),
        scope: String::new(),
        route: RATINGS_ENDPOINT.to_owned(),
        fetched_at,
        rating: rating.cloned(),
    })
    .unwrap_or_default()
}

fn validated_ratings(
    raw: &[u8],
    app_id: &str,
    schema: u32,
    repository: &str,
    scope: &str,
) -> Option<DetailRatingsCacheEnvelope> {
    let envelope: DetailRatingsCacheEnvelope = serde_json::from_slice(raw).ok()?;
    if envelope.schema != schema
        || envelope.app_id != app_id
        || envelope.repository != repository
        || envelope.scope != scope
        || envelope.route != RATINGS_ENDPOINT
    {
        return None;
    }
    if let Some(rating) = envelope.rating.as_ref() {
        validate_rating_counts(rating).ok()?;
    }
    Some(envelope)
}

fn read_legacy_flathub_ratings(
    cache: &crate::roots::CacheCapability,
    app_id: &str,
) -> Option<DetailRatingsCacheEnvelope> {
    // Both catalogue routes and deployed apps used to write separate ODRS slices.
    ["user", "system", "unknown", "--user"]
        .iter()
        .filter_map(|scope| {
            let name = legacy_flathub_ratings_name(app_id, scope);
            let raw = cache
                .read_legacy_catalog_cache(&name, DETAIL_RATINGS_CACHE_MAX_BYTES as u64)
                .ok()??;
            validated_ratings(&raw, app_id, 1, FLATHUB_REPOSITORY.origin, scope)
        })
        .max_by_key(|envelope| envelope.fetched_at)
}

#[cfg(test)]
#[test]
fn ratings_migrate_the_newest_stock_flathub_entry_without_fetching() {
    struct Offline;
    impl MetadataHttp for Offline {
        fn get(
            &self,
            _url: &str,
            _route: MetadataRoute,
            _cancellation: &OperationCancellation,
        ) -> Result<String, HttpFetchError> {
            panic!("fresh migrated ratings should satisfy the read");
        }
    }

    let cache = crate::roots::CacheCapability::for_test(
        &crate::test_scratch::path("legacy-flathub-ratings"),
    )
    .expect("ratings cache");
    let id = "org.gnome.Calculator";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let expected = fixture_ratings();
    assert!(cache.admit_detail_generation(1));
    for (scope, fetched_at, rating) in [
        ("user", now - 60, None),
        ("--user", now - 30, Some(&expected)),
    ] {
        let payload = serde_json::json!({
            "schema": 1,
            "app_id": id,
            "origin": FLATHUB_REPOSITORY.origin,
            "scope": scope,
            "route": RATINGS_ENDPOINT,
            "fetched_at": fetched_at,
            "rating": rating,
        });
        assert!(
            cache
                .publish_state_cache(&legacy_flathub_ratings_name(id, scope), 1, payload.to_string().as_bytes())
                .expect("legacy cache write"),
        );
    }
    assert_eq!(
        rating_with_http(
            Some(&cache),
            id,
            1,
            RATINGS_CACHE_TTL_SECS,
            false,
            &Offline,
            &OperationCancellation::default(),
        ),
        (Some(expected.clone()), DetailRouteState::Known),
    );
    let saved = cache
        .read_state_cache(&detail_ratings_cache_name(id), DETAIL_RATINGS_CACHE_MAX_BYTES)
        .expect("migrated cache read")
        .expect("repository-keyed cache entry");
    let row = validated_ratings(
        &saved, id, DETAIL_RATINGS_CACHE_SCHEMA, FLATHUB_REPOSITORY.url, "",
    )
    .expect("repository-keyed ratings");
    assert_eq!(row.fetched_at, now - 30);
    assert_eq!(row.rating, Some(expected));
}
