//! Error type for the storm crate.

use std::io;
use std::path::PathBuf;

/// Errors returned by the public API of `radar-storms`.
///
/// Callers branch on these: a [`StormsError::Manifest`] means the state is
/// corrupt and the caller must fail closed (spec-shared D7), while
/// [`StormsError::Io`] is usually retryable.
#[derive(Debug, thiserror::Error)]
pub enum StormsError {
    /// An environment variable could not be parsed.
    #[error("invalid configuration for {name}: {reason}")]
    Config {
        /// Environment variable name.
        name: &'static str,
        /// Why the value was rejected.
        reason: String,
    },
    /// The grid dimensions do not agree with the buffers passed in.
    #[error("invalid grid: {0}")]
    InvalidGrid(String),
    /// `manifest.json` exists but is not a usable manifest (spec-shared D7: fail closed).
    #[error("manifest unusable at {path}: {reason}")]
    Manifest {
        /// Manifest path.
        path: PathBuf,
        /// What is wrong with the document.
        reason: String,
    },
    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// JSON could not be serialized (for example a non-finite float).
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
