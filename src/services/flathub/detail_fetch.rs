// Detail context fetch and screenshot labels.

// Bundle the per-app Flathub endpoints through cancellable transports.
pub(crate) fn fetch_detail_context_cancellable(
    cache: Option<&crate::roots::CacheCapability>,
    app_id: &str,
    download_remote: Option<(&str, MutationScope)>,
    metadata_authorized: bool,
    generation: u64,
    cancellation: OperationCancellation,
) -> DetailContext {
    fetch_detail_context_with_seams(
        cache,
        app_id,
        download_remote,
        generation,
        metadata_authorized,
        fixture_enabled(),
        &ProcessFlatpakExecutor::cancellable(cancellation.clone()),
        &CurlMetadataHttp,
        &cancellation,
    )
}

// Verification retries must not replace release history, ratings or statistics.
pub(crate) fn fetch_publisher_verification_cancellable(
    app_id: &str,
    cancellation: OperationCancellation,
) -> PublisherVerification {
    fetch_publisher_verification(app_id, fixture_enabled(), &CurlMetadataHttp, &cancellation)
}

// The enrichment fan-out with every transport injected.
//
// Each step checks the owner's cancellation flag, and HTTP requests pass it
// into libcurl's progress callback. A dropped consumer therefore stops this
// worker within one transport poll interval instead of after the sum of the
// remaining step deadlines, and the caller receives the partial context
// filled so far with [`DetailContext::cancelled`] set.
#[allow(
    clippy::too_many_arguments,
    reason = "Keep captured network capabilities and cancellation identity explicit"
)]
fn fetch_detail_context_with_seams(
    cache: Option<&crate::roots::CacheCapability>,
    app_id: &str,
    download_remote: Option<(&str, MutationScope)>,
    generation: u64,
    metadata_authorized: bool,
    fixture: bool,
    executor: &dyn FlatpakExecutor,
    http: &dyn MetadataHttp,
    cancellation: &OperationCancellation,
) -> DetailContext {
    if fixture {
        return fixture_detail_context();
    }
    if !metadata_authorized {
        return DetailContext::unsupported_origin();
    }
    let mut ctx = DetailContext::default();
    macro_rules! stop_if_cancelled {
        () => {
            if cancellation.is_cancelled() {
                ctx.cancelled = true;
                return ctx;
            }
        };
    }

    stop_if_cancelled!();
    match flathub_app_info_with_http(app_id, fixture, http, cancellation) {
        Ok(info) => {
            ctx.description = strip_html(&info.description);
            ctx.license = info.project_license;
            ctx.license_kind = classify_license(&ctx.license);
            ctx.age_rating = format_age_rating(&info.content_rating);
            ctx.content_rating = info.content_rating;
            ctx.releases = info.releases;
            ctx.screenshot_count = info.screenshots.len();
            ctx.screenshot_candidates = info.screenshots;
            ctx.urls = info.urls;
            let eol_msg = if !info.is_eol_message.is_empty() {
                info.is_eol_message
            } else {
                info.end_of_life
            };
            ctx.is_eol = !eol_msg.is_empty();
            ctx.eol_message = eol_msg;
            ctx.metadata_state = if ctx.description.is_empty()
                && ctx.license.is_empty()
                && ctx.content_rating.is_empty()
                && ctx.releases.is_empty()
                && ctx.urls.is_empty()
                && ctx.screenshot_count == 0
                && ctx.eol_message.is_empty()
            {
                DetailRouteState::KnownEmpty
            } else {
                DetailRouteState::Known
            };
        }
        Err(_) => ctx.metadata_state = DetailRouteState::Failed,
    }
    stop_if_cancelled!();
    // A failed verification request remains unknown rather than becoming a negative verdict.
    ctx.verification = fetch_publisher_verification(app_id, fixture, http, cancellation);
    stop_if_cancelled!();
    match fetch_app_stats_with_http(app_id, fixture, http, cancellation) {
        Ok(stats) => {
            ctx.installs_last_month = stats.installs_last_month;
            ctx.stats_state = if stats.installs_last_month == 0 {
                DetailRouteState::KnownEmpty
            } else {
                DetailRouteState::Known
            };
        }
        Err(_) => ctx.stats_state = DetailRouteState::Failed,
    }

    // Download size: try `flatpak remote-info --show-commit` to get
    // the pre-install download size. This works even for uninstalled apps.
    stop_if_cancelled!();
    if let Some((remote, scope)) = download_remote
        && let Ok(size) =
            fetch_download_size_for_remote_with_executor(app_id, remote, scope, fixture, executor)
    {
        ctx.download_size = size;
    }

    // Last updated: use the newest release timestamp.
    ctx.last_updated = newest_release_date(&ctx.releases);

    // The ODRS endpoint returns a table for all apps, but only this request's
    // bounded, identity-keyed histogram is retained on disk.
    stop_if_cancelled!();
    let (ratings, ratings_state) = rating_with_http(
        cache,
        app_id,
        generation,
        RATINGS_CACHE_TTL_SECS,
        fixture,
        http,
        cancellation,
    );
    ctx.ratings = ratings;
    ctx.ratings_state = ratings_state;

    stop_if_cancelled!();
    ctx
}

/// How long a cached ODRS ratings table stays fresh. Six hours: the table is
/// ~500KB and star counts move on the order of days, so refetching more often
/// spends bandwidth to change nothing.
pub(crate) const RATINGS_CACHE_TTL_SECS: u64 = 6 * 60 * 60;

// Pick this app's histogram out of a fetched ODRS table.
//
// Split from the detail-context pipeline so the wiring is testable without a
// network call: everything about the ratings step except "where did the table
// come from" lives here. `None` table (both cache and fetch failed) and
// "table has no entry for this app" both yield `None`, which the detail page
// renders as the attribution line alone.
fn ratings_for(
    app_id: &str,
    table: Option<&std::collections::HashMap<String, RatingsCounts>>,
) -> Option<RatingsCounts> {
    lookup_rating(table?, app_id).cloned()
}

fn rating_with_http(
    cache: Option<&crate::roots::CacheCapability>,
    app_id: &str,
    generation: u64,
    ttl_secs: u64,
    fixture: bool,
    http: &dyn MetadataHttp,
    cancellation: &OperationCancellation,
) -> (Option<RatingsCounts>, DetailRouteState) {
    if fixture {
        let rating = ratings_for(
            app_id,
            fetch_all_ratings_with_http(fixture, http, cancellation)
                .ok()
                .as_ref(),
        );
        let state = if rating.is_some() {
            DetailRouteState::Known
        } else {
            DetailRouteState::KnownEmpty
        };
        return (rating, state);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let name = detail_ratings_cache_name(app_id);
    let cached = match cache {
        Some(cache) => match cache.read_state_cache(&name, DETAIL_RATINGS_CACHE_MAX_BYTES) {
            Ok(Some(raw)) => match validated_ratings(
                &raw, app_id, DETAIL_RATINGS_CACHE_SCHEMA, FLATHUB_REPOSITORY.url, "",
            ) {
                Some(cached) => Some(cached),
                None => return (None, DetailRouteState::Failed),
            },
            Ok(None) => read_legacy_flathub_ratings(cache, app_id).inspect(|legacy| {
                let payload = detail_ratings_payload(app_id, legacy.fetched_at, legacy.rating.as_ref());
                if !payload.is_empty() && !cancellation.is_cancelled() {
                    let _ = cache.publish_state_cache(&name, generation, &payload);
                }
            }),
            Err(_) => return (None, DetailRouteState::Failed),
        },
        None => None,
    };
    let cached = cached.map(|row| (row.rating, now.saturating_sub(row.fetched_at)));
    if let Some((rating, age_secs)) = cached.as_ref()
        && *age_secs <= ttl_secs
    {
        let state = if rating.is_some() {
            DetailRouteState::Known
        } else {
            DetailRouteState::KnownEmpty
        };
        return (rating.clone(), state);
    }

    let fetched = match fetch_all_ratings_with_http(fixture, http, cancellation) {
        Ok(table) => ratings_for(app_id, Some(&table)),
        Err(_) => {
            return match cached {
                Some((rating, age_secs)) => (rating, DetailRouteState::CachedStale { age_secs }),
                None => (None, DetailRouteState::Failed),
            };
        }
    };
    if cancellation.is_cancelled() {
        return (None, DetailRouteState::Failed);
    }
    if let Some(cache) = cache {
        let payload = detail_ratings_payload(app_id, now, fetched.as_ref());
        if !payload.is_empty() && !cancellation.is_cancelled() {
            match cache.publish_state_cache(&name, generation, &payload) {
                Ok(true) => {}
                Ok(false) | Err(_) => return (fetched, DetailRouteState::Failed),
            }
        }
    }
    let state = if fetched.is_some() {
        DetailRouteState::Known
    } else {
        DetailRouteState::KnownEmpty
    };
    (fetched, state)
}

pub(crate) fn no_screenshots_label() -> &'static str {
    "No screenshots"
}

/// The accessible name for the "no screenshots" empty state — the fuller
/// sentence, distinct from [`no_screenshots_label`].
pub(crate) fn no_screenshots_aria() -> &'static str {
    "No screenshots available"
}
