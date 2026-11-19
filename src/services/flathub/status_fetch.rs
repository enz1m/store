// Unknown verification failures remain distinct from confirmed publisher status.
fn fetch_publisher_verification(
    app_id: &str,
    fixture: bool,
    http: &dyn MetadataHttp,
    cancellation: &OperationCancellation,
) -> PublisherVerification {
    if fixture {
        return PublisherVerification::Verified {
            method: "website".into(),
            provenance: VerificationProvenance::Live,
        };
    }
    if crate::flatpak_args::validate_app_id(app_id).is_err() {
        return PublisherVerification::unavailable(VerificationFailure::NotRequested);
    }
    let url = format!("https://flathub.org/api/v2/verification/{app_id}/status");
    let raw = match http.get(&url, MetadataRoute::Verification, cancellation) {
        Ok(raw) => raw,
        Err(HttpFetchError::Cancelled) => {
            return PublisherVerification::unavailable(VerificationFailure::Cancelled);
        }
        Err(HttpFetchError::HttpStatus { status }) => {
            return PublisherVerification::unavailable(VerificationFailure::Status(
                status.to_string(),
            ));
        }
        Err(HttpFetchError::PolicyBlocked { .. }) => {
            return PublisherVerification::unavailable(VerificationFailure::Policy);
        }
        Err(other) => {
            return PublisherVerification::unavailable(VerificationFailure::Transport(
                other.to_string(),
            ));
        }
    };
    // A cancellation that lands between the read and the parse still belongs
    // to the newer load, not to this one.
    if cancellation.is_cancelled() {
        return PublisherVerification::unavailable(VerificationFailure::Cancelled);
    }
    match parse_verification_status(&raw) {
        Ok(status) if status.verified => PublisherVerification::Verified {
            method: status.method,
            provenance: VerificationProvenance::Live,
        },
        Ok(_) => PublisherVerification::KnownUnverified {
            provenance: VerificationProvenance::Live,
        },
        Err(detail) => PublisherVerification::unavailable(VerificationFailure::Schema(detail)),
    }
}

fn fetch_app_stats_with_http(
    app_id: &str,
    fixture: bool,
    http: &dyn MetadataHttp,
    cancellation: &OperationCancellation,
) -> Result<FlathubAppStats, String> {
    if fixture {
        return Ok(FlathubAppStats {
            installs_total: 3_200_000,
            installs_last_month: 612_000,
            installs_last_7_days: 141_000,
        });
    }
    crate::flatpak_args::validate_app_id(app_id)?;
    let url = format!("https://flathub.org/api/v2/stats/{app_id}");
    let raw = http
        .get(&url, MetadataRoute::Stats, cancellation)
        .map_err(|error| error.to_string())?;
    parse_app_stats(&raw)
}

fn fetch_download_size_for_remote_with_executor(
    app_id: &str,
    origin: &str,
    scope: MutationScope,
    fixture: bool,
    executor: &dyn FlatpakExecutor,
) -> Result<u64, String> {
    if fixture {
        return Ok(212_000_000);
    }
    let mut args = crate::flatpak_args::FlatpakArgs::new("remote-info");
    args.options([scope.option(), "--show-commit"]);
    args.remote(origin)?;
    args.app_id(app_id)?;
    let raw = run_flatpak_args_with(executor, &args.finish())?;
    parse_download_size(&raw).ok_or_else(|| "no download size found".to_owned())
}

/// Parse the `Download size:` line from `flatpak remote-info --show-commit`
/// output. Returns the size in bytes.
///
/// Example lines:
///   `"Download size: 42302976 (40.3\u{a0}MB)"`
///   `"Download size: 1234567"`
///   `"Installed size: 123456789 (117.7\u{a0}MB)"`
pub(crate) fn parse_download_size(raw: &str) -> Option<u64> {
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Download size:") {
            let rest = rest.trim();
            // Take the first token (the byte count before any parenthesised human string).
            let num_str = rest.split_whitespace().next()?;
            return num_str.parse::<u64>().ok();
        }
    }
    // Fall back to Installed size if Download size wasn't emitted.
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Installed size:") {
            let rest = rest.trim();
            let num_str = rest.split_whitespace().next()?;
            return num_str.parse::<u64>().ok();
        }
    }
    None
}

/// Format download size with the shared decimal SI policy; zero leaves the cell blank.
pub(crate) fn format_byte_size(bytes: u64) -> String {
    if bytes == 0 {
        return String::new();
    }
    enzui_widgets::file_browser::format_bytes(bytes)
}

/// Return the formatted date of the newest release, or empty if no releases.
pub(crate) fn newest_release_date(releases: &[FlathubRelease]) -> String {
    // Releases are already ordered newest-first by Flathub;
    // just take the first one with a non-empty timestamp.
    releases
        .iter()
        .find(|r| !r.timestamp.is_empty())
        .map(FlathubRelease::formatted_date)
        .unwrap_or_default()
}
