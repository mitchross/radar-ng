//! Error type for the crate. Callers branch on the variants that matter to ingest
//! (HTTP 404 means "pending" for HRRR and NAQFC; a range that is not honoured must fail).

use thiserror::Error;

/// Errors from GRIB2 decoding, `.idx` parsing, S3 listing and HTTP fetches.
#[derive(Debug, Error)]
pub enum GribError {
    /// The GRIB2 byte stream could not be parsed.
    #[error("GRIB2 parse error: {0}")]
    Parse(String),
    /// The message uses a template or option this crate does not decode.
    #[error("unsupported GRIB2 feature: {0}")]
    Unsupported(String),
    /// A value could not be decoded (bitmap/data inconsistency and similar).
    #[error("GRIB2 decode error: {0}")]
    Decode(String),
    /// An `.idx` line was malformed (non-integer num or offset). Python raises ValueError here.
    #[error("malformed .idx line {line:?}: {detail}")]
    IdxParse { line: String, detail: String },
    /// An S3 ListObjectsV2 response could not be parsed.
    #[error("malformed S3 listing: {0}")]
    ListingParse(String),
    /// The HTTP status was not the success status the call requires.
    #[error("HTTP {status} from {url}")]
    HttpStatus { status: u16, url: String },
    /// A transport-level failure (DNS, TLS, connect, timeout, body read).
    #[error("transport error for {url}: {source}")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    /// A ranged GET did not return 206 Partial Content with the requested bytes (PLAN defect D5).
    #[error("range request {range} to {url} was not honoured: {detail}")]
    RangeNotHonored {
        url: String,
        range: String,
        detail: String,
    },
    /// An object key does not carry a parseable valid time.
    #[error("object key {key:?} has no parseable timestamp: {detail}")]
    KeyTimestamp { key: String, detail: String },
    /// Local I/O (gunzip input, temp files).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl GribError {
    /// True for an HTTP 404 response. HRRR treats this as "hour pending", not as a failure.
    pub fn is_not_found(&self) -> bool {
        matches!(self, GribError::HttpStatus { status: 404, .. })
    }

    /// Converts a reqwest error into a [`GribError`], keeping the URL it was for.
    pub(crate) fn transport(url: &str, source: reqwest::Error) -> Self {
        if let Some(status) = source.status() {
            return GribError::HttpStatus {
                status: status.as_u16(),
                url: url.to_string(),
            };
        }
        GribError::Transport {
            url: url.to_string(),
            source,
        }
    }
}

/// Crate result alias.
pub type Result<T, E = GribError> = std::result::Result<T, E>;
