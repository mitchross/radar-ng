//! Error types for the numerical core.
//!
//! The variants map onto the pysteps failure modes that radar-ng turns into
//! `degraded / pysteps_failed` (spec section 7.10). The glue decides the status; this
//! crate only reports what failed and where.

use thiserror::Error;

use crate::ar::ArError;

/// Failure of the nowcast numerics.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum NowcastError {
    /// Inputs have the wrong shape, count or parameters (pysteps `ValueError`).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The motion field contains NaN or infinite values (pysteps `ValueError`).
    #[error("velocity contains non-finite values")]
    NonFiniteVelocity,
    /// The Yule-Walker fit failed at a cascade level (pysteps `LinAlgError`).
    #[error("AR(2) fit failed at cascade level {level}: {source}")]
    ArFit {
        /// Zero-based cascade level.
        level: usize,
        /// Underlying solver failure.
        #[source]
        source: ArError,
    },
    /// The fitted AR(2) process is not stationary (pysteps `RuntimeError`).
    #[error("non-stationary AR(2) process at cascade level {level}")]
    NonStationary {
        /// Zero-based cascade level.
        level: usize,
    },
    /// Probability matching could not run (pysteps `ValueError`).
    #[error("probability matching failed: {0}")]
    ProbMatch(String),
}
