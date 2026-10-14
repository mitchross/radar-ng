//! Error type shared by every tiler module.
//!
//! Callers branch on the variant, not on message text: `Unbound` means "a pre-v2 tree
//! needs Python adoption", `Conflict` means "the immutable path belongs to other inputs",
//! `Corrupt` means fail-closed on-disk inconsistency.

use std::io;

/// Errors produced by radar-tiler.
#[derive(Debug, thiserror::Error)]
pub enum TilerError {
    /// Caller-supplied input is invalid (shapes, ranges, palette names, options).
    #[error("invalid input: {0}")]
    Invalid(String),
    /// A complete pyramid or the publish protocol refuses to proceed (identity conflict).
    #[error("pyramid identity conflict: {0}")]
    Conflict(String),
    /// A non-empty pre-v2 (unmarked) pyramid sits at a final path. Rust never adopts such
    /// trees (spec-tiler H6); adoption stays with the Python publisher.
    #[error("unbound pyramid requires Python adoption: {0}")]
    Unbound(String),
    /// On-disk state is inconsistent (marker mismatch, staged tile count, bad PNG, ...).
    #[error("corrupt pyramid: {0}")]
    Corrupt(String),
    /// Underlying I/O failure.
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
}

/// Crate result alias.
pub type Result<T, E = TilerError> = std::result::Result<T, E>;
