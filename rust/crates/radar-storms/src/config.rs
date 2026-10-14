//! Runtime configuration read from the same environment variables as the Python
//! modules (spec-shared section 0.5). Defaults are the Python defaults.

use std::path::PathBuf;

use crate::error::StormsError;

/// Tunables for storm detection, tracking and prefetch planning.
#[derive(Debug, Clone, PartialEq)]
pub struct StormConfig {
    /// `STORM_THRESHOLD_DBZ` (default 40): reflectivity at or above which a pixel is in a cell.
    pub threshold_dbz: f64,
    /// `STORM_MIN_PIXELS` (default 5): components smaller than this are dropped.
    pub min_pixels: usize,
    /// `STORM_MAX_CELLS` (default 500): cap on features kept per frame.
    pub max_storms: usize,
    /// `STORM_MAX_TRACK_SPEED_KMH` (default 160): speed clamp and gate term.
    pub max_track_speed_kmh: f64,
    /// `STORM_MATCH_SLACK_KM` (default 8): association gate floor.
    pub match_slack_km: f64,
    /// `STORM_TRACK_HISTORY_MIN` (default 20): history window in minutes.
    pub track_history_min: f64,
    /// `STORM_MIN_TRACK_SPAN_MIN` (default 5): minimum history span before motion is fitted.
    pub min_track_span_min: f64,
    /// `STORM_PREFETCH_PADDING_KM` (default 12): bbox padding around each cell.
    pub prefetch_padding_km: f64,
    /// `STORM_PREFETCH_MAX_DISTANCE_KM` (default 500): max user-to-cell distance for a plan.
    pub prefetch_max_distance_km: f64,
}

impl Default for StormConfig {
    fn default() -> Self {
        Self {
            threshold_dbz: 40.0,
            min_pixels: 5,
            max_storms: 500,
            max_track_speed_kmh: 160.0,
            match_slack_km: 8.0,
            track_history_min: 20.0,
            min_track_span_min: 5.0,
            prefetch_padding_km: 12.0,
            prefetch_max_distance_km: 500.0,
        }
    }
}

impl StormConfig {
    /// Read the configuration from the process environment, falling back to the defaults.
    ///
    /// # Errors
    /// Returns [`StormsError::Config`] when a variable is set but does not parse.
    pub fn from_env() -> Result<Self, StormsError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Build the configuration from an arbitrary lookup (used by tests).
    ///
    /// # Errors
    /// Returns [`StormsError::Config`] when a provided value does not parse.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, StormsError> {
        let d = Self::default();
        Ok(Self {
            threshold_dbz: float_var(&get, "STORM_THRESHOLD_DBZ", d.threshold_dbz)?,
            min_pixels: uint_var(&get, "STORM_MIN_PIXELS", d.min_pixels)?,
            max_storms: uint_var(&get, "STORM_MAX_CELLS", d.max_storms)?,
            max_track_speed_kmh: float_var(
                &get,
                "STORM_MAX_TRACK_SPEED_KMH",
                d.max_track_speed_kmh,
            )?,
            match_slack_km: float_var(&get, "STORM_MATCH_SLACK_KM", d.match_slack_km)?,
            track_history_min: float_var(&get, "STORM_TRACK_HISTORY_MIN", d.track_history_min)?,
            min_track_span_min: float_var(&get, "STORM_MIN_TRACK_SPAN_MIN", d.min_track_span_min)?,
            prefetch_padding_km: float_var(
                &get,
                "STORM_PREFETCH_PADDING_KM",
                d.prefetch_padding_km,
            )?,
            prefetch_max_distance_km: float_var(
                &get,
                "STORM_PREFETCH_MAX_DISTANCE_KM",
                d.prefetch_max_distance_km,
            )?,
        })
    }
}

/// Grid root from `GRID_DIR` (default `/data/grids`).
pub fn grid_dir_from_env() -> PathBuf {
    std::env::var("GRID_DIR").map_or_else(|_| PathBuf::from("/data/grids"), PathBuf::from)
}

/// State root from `STATE_DIR` (default `/data/state`).
pub fn state_dir_from_env() -> PathBuf {
    std::env::var("STATE_DIR").map_or_else(|_| PathBuf::from("/data/state"), PathBuf::from)
}

fn float_var(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: f64,
) -> Result<f64, StormsError> {
    match get(name) {
        None => Ok(default),
        Some(raw) => raw.trim().parse::<f64>().map_err(|e| StormsError::Config {
            name,
            reason: e.to_string(),
        }),
    }
}

fn uint_var(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: usize,
) -> Result<usize, StormsError> {
    match get(name) {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse::<usize>()
            .map_err(|e| StormsError::Config {
                name,
                reason: e.to_string(),
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_python_module_constants() {
        let c = StormConfig::from_lookup(|_| None).unwrap();
        assert_eq!(c, StormConfig::default());
        assert_eq!(c.threshold_dbz, 40.0);
        assert_eq!(c.min_pixels, 5);
        assert_eq!(c.max_storms, 500);
        assert_eq!(c.max_track_speed_kmh, 160.0);
        assert_eq!(c.match_slack_km, 8.0);
        assert_eq!(c.track_history_min, 20.0);
        assert_eq!(c.min_track_span_min, 5.0);
        assert_eq!(c.prefetch_padding_km, 12.0);
        assert_eq!(c.prefetch_max_distance_km, 500.0);
    }

    #[test]
    fn env_values_override_and_bad_values_are_errors() {
        let c =
            StormConfig::from_lookup(|n| (n == "STORM_THRESHOLD_DBZ").then(|| " 35.5 ".to_owned()))
                .unwrap();
        assert_eq!(c.threshold_dbz, 35.5);
        let err =
            StormConfig::from_lookup(|n| (n == "STORM_MIN_PIXELS").then(|| "five".to_owned()));
        assert!(matches!(
            err,
            Err(StormsError::Config {
                name: "STORM_MIN_PIXELS",
                ..
            })
        ));
    }
}
