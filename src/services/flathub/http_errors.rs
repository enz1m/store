// HTTP fetch errors and catalog parse errors.

/// Maximum retained response bytes for the full catalogue and ranking rails.
pub(crate) const CATALOG_BODY_MAX_BYTES: usize = 24 * 1024 * 1024;
/// Maximum retained response bytes for the all-app ODRS ratings table.
pub(crate) const RATINGS_BODY_MAX_BYTES: usize = 16 * 1024 * 1024;
/// Maximum retained response bytes for one app's metadata, stats, or status.
pub(crate) const PER_APP_BODY_MAX_BYTES: usize = 2 * 1024 * 1024;
const HTTP_CONNECT_DEADLINE_MS: u64 = 10_000;
const HTTP_READ_DEADLINE_MS: u64 = 10_000;
const HTTP_TOTAL_DEADLINE_MS: u64 = 10_000;
const BULK_TOTAL_DEADLINE_MS: u64 = 120_000;
const FLATHUB_ORIGIN_HOST: &str = "flathub.org";
const ENZIMOS_ORIGIN_HOST: &str = "flatpak.enzim.dev";
const ODRS_ORIGIN_HOST: &str = "odrs.gnome.org";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MetadataRoute {
    Catalogue,
    EnzimosCatalogue,
    Ratings,
    Appstream,
    Verification,
    Stats,
}

impl MetadataRoute {
    fn body_limit(self) -> usize {
        match self {
            Self::Catalogue => CATALOG_BODY_MAX_BYTES,
            Self::Ratings => RATINGS_BODY_MAX_BYTES,
            Self::Appstream | Self::Verification | Self::Stats | Self::EnzimosCatalogue => {
                PER_APP_BODY_MAX_BYTES
            }
        }
    }

    fn origin_host(self) -> &'static str {
        match self {
            Self::Catalogue | Self::Appstream | Self::Verification | Self::Stats => {
                FLATHUB_ORIGIN_HOST
            }
            Self::Ratings => ODRS_ORIGIN_HOST,
            Self::EnzimosCatalogue => ENZIMOS_ORIGIN_HOST,
        }
    }

    fn transport_limits(self) -> TransportLimits {
        if matches!(self, Self::EnzimosCatalogue) {
            return TransportLimits {
                max_bytes: self.body_limit() as u64,
                connect_timeout_ms: 2_000,
                read_timeout_ms: 2_000,
                total_timeout_ms: 4_000,
            };
        }
        TransportLimits {
            max_bytes: u64::try_from(self.body_limit()).unwrap_or(u64::MAX),
            connect_timeout_ms: HTTP_CONNECT_DEADLINE_MS,
            read_timeout_ms: HTTP_READ_DEADLINE_MS,
            // The catalogue (~3.4 MB, uncompressed) and ratings tables cannot
            // finish in 10 s on slower links; the 10 s idle read timeout still
            // catches stalls.
            total_timeout_ms: match self {
                Self::Catalogue | Self::Ratings => BULK_TOTAL_DEADLINE_MS,
                _ => HTTP_TOTAL_DEADLINE_MS,
            },
        }
    }
}

/// Typed failures from the Flathub/ODRS metadata transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HttpFetchError {
    /// The owning consumer went away while the request was in flight.
    Cancelled,
    /// The complete response body exceeded the route's inclusive ceiling.
    BodyTooLarge { limit: usize, received: usize },
    /// The complete response body was not UTF-8.
    InvalidUtf8,
    /// A connect, read-idle, or total request deadline expired.
    DeadlineExceeded { milliseconds: u64 },
    /// URL, redirect, DNS-address, or connected-peer policy refused the request.
    PolicyBlocked { code: &'static str },
    /// Response-header accumulation exceeded the framework ceiling.
    HeaderTooLarge { code: &'static str },
    /// The remote service returned a non-success HTTP status.
    HttpStatus { status: u16 },
    /// TLS setup or peer verification failed.
    Tls { code: u32 },
    /// A non-policy transport operation failed.
    Transport {
        code: &'static str,
        backend_code: Option<u32>,
    },
}

impl std::fmt::Display for HttpFetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("metadata request cancelled"),
            Self::BodyTooLarge { limit, received } => write!(
                formatter,
                "metadata body limit exceeded: {received}>{limit}"
            ),
            Self::InvalidUtf8 => formatter.write_str("metadata body is not UTF-8"),
            Self::DeadlineExceeded { milliseconds } => {
                write!(formatter, "metadata timeout: {milliseconds}ms")
            }
            Self::PolicyBlocked { code } => write!(formatter, "metadata policy blocked: {code}"),
            Self::HeaderTooLarge { code } => {
                write!(formatter, "metadata header limit exceeded: {code}")
            }
            Self::HttpStatus { status } => write!(formatter, "metadata HTTP status: {status}"),
            Self::Tls { code } => write!(formatter, "metadata TLS failure: {code}"),
            Self::Transport { code, backend_code } => match backend_code {
                Some(backend_code) => {
                    write!(
                        formatter,
                        "metadata transport failure: {code}/{backend_code}"
                    )
                }
                None => write!(formatter, "metadata transport failure: {code}"),
            },
        }
    }
}

impl std::error::Error for HttpFetchError {}

// The bounded, user-safe class of a metadata transport failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HttpErrorCategory {
    Cancelled,
    Policy,
    Status,
    Header,
    Payload,
    Encoding,
    Tls,
    Timeout,
    Transport,
}

pub(crate) const CANCELLED_SUMMARY: &str = "The catalogue refresh was cancelled.";
pub(crate) const POLICY_SUMMARY: &str =
    "The catalogue request was blocked by network safety policy.";
pub(crate) const STATUS_SUMMARY: &str = "Flathub returned an error for the catalogue request.";
pub(crate) const HEADER_SUMMARY: &str =
    "The Flathub catalogue response headers were too large to read.";
pub(crate) const PAYLOAD_SUMMARY: &str = "The Flathub catalogue response was too large to read.";
pub(crate) const ENCODING_SUMMARY: &str = "The Flathub catalogue response was not valid text.";
pub(crate) const TLS_SUMMARY: &str = "Flathub's secure connection could not be verified.";
pub(crate) const TIMEOUT_SUMMARY: &str = "The Flathub catalogue request timed out.";
pub(crate) const NETWORK_SUMMARY: &str = "Flathub could not be reached over the network.";

impl HttpFetchError {
    pub(crate) fn category(&self) -> HttpErrorCategory {
        match self {
            Self::Cancelled => HttpErrorCategory::Cancelled,
            Self::PolicyBlocked { .. } => HttpErrorCategory::Policy,
            Self::HttpStatus { .. } => HttpErrorCategory::Status,
            Self::HeaderTooLarge { .. } => HttpErrorCategory::Header,
            Self::BodyTooLarge { .. } => HttpErrorCategory::Payload,
            Self::InvalidUtf8 => HttpErrorCategory::Encoding,
            Self::Tls { .. } => HttpErrorCategory::Tls,
            Self::DeadlineExceeded { .. } => HttpErrorCategory::Timeout,
            Self::Transport { .. } => HttpErrorCategory::Transport,
        }
    }

    // Static user-facing copy with no transport-controlled detail.
    pub(crate) fn user_summary(&self) -> &'static str {
        match self.category() {
            HttpErrorCategory::Cancelled => CANCELLED_SUMMARY,
            HttpErrorCategory::Policy => POLICY_SUMMARY,
            HttpErrorCategory::Status => STATUS_SUMMARY,
            HttpErrorCategory::Header => HEADER_SUMMARY,
            HttpErrorCategory::Payload => PAYLOAD_SUMMARY,
            HttpErrorCategory::Encoding => ENCODING_SUMMARY,
            HttpErrorCategory::Tls => TLS_SUMMARY,
            HttpErrorCategory::Timeout => TIMEOUT_SUMMARY,
            HttpErrorCategory::Transport => NETWORK_SUMMARY,
        }
    }

    // Fixed category/code diagnostic with no request or response data.
    pub(crate) fn diagnostic(&self) -> String {
        self.to_string()
    }
}

// The bounded, user-safe class of a catalogue **payload** parse failure.
//
// A `serde_json::Error` `Display` carries parser vocabulary, line/column
// coordinates, and — for a deserialize/`Data` fault — echoed field/variant
// names lifted from an attacker-controlled body, unbounded and able to smuggle
// control or bidi runs. Every parse failure collapses to exactly one of these
// classes so Discover can state what went wrong without interpolating any of
// that. Raw detail survives only through [`CatalogParseError::diagnostic`] for
// an escaped log, mirroring [`HttpFetchError`]'s transport boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CatalogParseCategory {
    /// The bytes were not valid JSON at all: a syntax fault, a truncated body,
    /// or control/bidi noise where a token was expected.
    InvalidDocument,
    /// The bytes parsed as JSON but did not match the catalogue schema: a
    /// wrong type, a missing required field, or an out-of-range value.
    UnsupportedSchema,
}

// The exact user-visible summary for each [`CatalogParseCategory`].
pub(crate) const INVALID_DOCUMENT_SUMMARY: &str =
    "The Flathub catalogue response was not readable.";
pub(crate) const UNSUPPORTED_SCHEMA_SUMMARY: &str =
    "The Flathub catalogue response was in an unrecognised format.";

/// A typed catalogue payload parse failure.
///
/// The `category` is the only value a UI may read; `detail` is the raw
/// `serde_json` `Display`, retained solely for the escaped diagnostic sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CatalogParseError {
    category: CatalogParseCategory,
    detail: String,
}

impl CatalogParseError {
    // Classify a `serde_json::Error` into a bounded category and capture its
    // raw detail for escaped logging only.
    fn from_serde(error: &serde_json::Error) -> Self {
        let category = match error.classify() {
            // A well-formed JSON document whose shape/typing the schema rejects
            // is the one actionable-distinct class: the transport worked, the
            // format did not.
            serde_json::error::Category::Data => CatalogParseCategory::UnsupportedSchema,
            // Syntax, Eof, and the (unreachable for `from_str`) Io fault are all
            // "these bytes are not our document".
            serde_json::error::Category::Syntax
            | serde_json::error::Category::Eof
            | serde_json::error::Category::Io => CatalogParseCategory::InvalidDocument,
        };
        Self {
            category,
            detail: error.to_string(),
        }
    }

    // The bounded, user-safe class of this failure.
    pub(crate) fn category(&self) -> CatalogParseCategory {
        self.category
    }

    // Return the exact bounded, single-line summary for this failure class.
    //
    // Only static copy is returned; the raw `detail` is never UI copy.
    pub(crate) fn user_summary(&self) -> &'static str {
        match self.category {
            CatalogParseCategory::InvalidDocument => INVALID_DOCUMENT_SUMMARY,
            CatalogParseCategory::UnsupportedSchema => UNSUPPORTED_SCHEMA_SUMMARY,
        }
    }

    // The raw `serde_json` diagnostic for an escaped log, never a UI surface.
    pub(crate) fn diagnostic(&self) -> &str {
        &self.detail
    }
}

impl std::fmt::Display for CatalogParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.user_summary())
    }
}

impl std::error::Error for CatalogParseError {}

trait MetadataTransport {
    fn fetch(
        &self,
        request: &RemoteRequest,
        policy: &UrlPolicy,
        limits: &TransportLimits,
        probe: &CancelProbe<'_>,
        progress: &mut dyn FnMut(TransferProgress),
    ) -> Result<TransportResponse, RemoteTransportError>;
}
