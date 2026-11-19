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

const DETAIL_RATINGS_CACHE_SCHEMA: u32 = 1;
const RATINGS_ENDPOINT: &str = "https://odrs.gnome.org/1.0/reviews/api/ratings";

#[derive(serde::Serialize, serde::Deserialize)]
struct DetailRatingsCacheEnvelope {
    schema: u32,
    app_id: String,
    origin: String,
    scope: String,
    route: String,
    fetched_at: u64,
    rating: Option<RatingsCounts>,
}

fn detail_ratings_cache_name(app_id: &str, origin: &str, scope: &str) -> Option<String> {
    crate::flatpak_args::validate_app_id(app_id).ok()?;
    crate::flatpak_args::validate_remote_name(origin).ok()?;
    if scope.len() > 256 {
        return None;
    }
    let identity = format!(
        "schema={DETAIL_RATINGS_CACHE_SCHEMA}\norigin={origin}\nscope={scope}\nroute={RATINGS_ENDPOINT}\napp={app_id}"
    );
    Some(format!(
        "detail-ratings-v{DETAIL_RATINGS_CACHE_SCHEMA}-{}.json",
        cache_identity(CacheIdentityDomain::HostId, identity.as_bytes())
    ))
}

fn detail_ratings_payload(
    app_id: &str,
    origin: &str,
    scope: &str,
    fetched_at: u64,
    rating: Option<&RatingsCounts>,
) -> Vec<u8> {
    serde_json::to_vec(&DetailRatingsCacheEnvelope {
        schema: DETAIL_RATINGS_CACHE_SCHEMA,
        app_id: app_id.to_owned(),
        origin: origin.to_owned(),
        scope: scope.to_owned(),
        route: RATINGS_ENDPOINT.to_owned(),
        fetched_at,
        rating: rating.cloned(),
    })
    .unwrap_or_default()
}

fn detail_ratings_from_slice(
    raw: &[u8],
    app_id: &str,
    origin: &str,
    scope: &str,
    now: u64,
) -> Option<(Option<RatingsCounts>, u64)> {
    let envelope: DetailRatingsCacheEnvelope = serde_json::from_slice(raw).ok()?;
    if envelope.schema != DETAIL_RATINGS_CACHE_SCHEMA
        || envelope.app_id != app_id
        || envelope.origin != origin
        || envelope.scope != scope
        || envelope.route != RATINGS_ENDPOINT
    {
        return None;
    }
    if let Some(rating) = envelope.rating.as_ref() {
        validate_rating_counts(rating).ok()?;
    }
    Some((envelope.rating, now.saturating_sub(envelope.fetched_at)))
}
