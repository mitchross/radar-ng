//! Lightning configuration: endpoints, bounding box and buffer knobs (spec-ingest-aux section 6.1 and 6.2).

use std::path::{Path, PathBuf};
use std::time::Duration;

use radar_core::env::{EnvError, float_var, int_var, path_var, var};

/// Primary Blitzortung hosts, verified 2026-09-29 as accepting `wss://` on port 443 (spec 6.1).
pub const PRIMARY_HOSTS: [&str; 4] = ["ws1", "ws2", "ws7", "ws8"];

/// Legacy plain-`ws` ports. Every one refused connections when verified; they are a last resort.
pub const LEGACY_PORTS: [u16; 4] = [8087, 8088, 8089, 8090];

/// Default primary endpoints, in code order.
#[must_use]
pub fn primary_endpoints() -> Vec<String> {
    PRIMARY_HOSTS
        .iter()
        .map(|host| format!("wss://{host}.blitzortung.org:443/"))
        .collect()
}

/// Default legacy endpoints: hosts `ws1`..`ws8`, ports 8087..8090, host outer and port inner (spec 6.1).
#[must_use]
pub fn legacy_endpoints() -> Vec<String> {
    (1..=8)
        .flat_map(|i| {
            LEGACY_PORTS
                .iter()
                .map(move |port| format!("ws://ws{i}.blitzortung.org:{port}/"))
        })
        .collect()
}

/// Parse the `LIGHTNING_WS_ENDPOINTS` override: comma-separated, each entry trimmed, empties dropped.
#[must_use]
pub fn parse_endpoint_override(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Inclusive latitude and longitude box for accepted strikes (spec 6.7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    /// South edge (`LIGHTNING_LAT_MIN`, default 15).
    pub lat_min: f64,
    /// North edge (`LIGHTNING_LAT_MAX`, default 55).
    pub lat_max: f64,
    /// West edge (`LIGHTNING_LON_MIN`, default -130).
    pub lon_min: f64,
    /// East edge (`LIGHTNING_LON_MAX`, default -50).
    pub lon_max: f64,
}

impl Default for BoundingBox {
    fn default() -> Self {
        Self {
            lat_min: 15.0,
            lat_max: 55.0,
            lon_min: -130.0,
            lon_max: -50.0,
        }
    }
}

impl BoundingBox {
    /// True when `(lat, lon)` lies inside the box, edges included. NaN is never inside.
    #[must_use]
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        self.lat_min <= lat && lat <= self.lat_max && self.lon_min <= lon && lon <= self.lon_max
    }
}

/// Everything `lightning_consume_stream` needs. Build it once per worker from the environment.
#[derive(Debug, Clone, PartialEq)]
pub struct LightningConfig {
    /// `STATE_DIR`. The output file is `<state_dir>/lightning.json`.
    pub state_dir: PathBuf,
    /// `LIGHTNING_RETENTION_MIN` (default 15): rolling window in minutes.
    pub retention_min: i64,
    /// `LIGHTNING_FLUSH_S` (default 2.0): minimum seconds between strike-driven flushes.
    pub flush_every_s: f64,
    /// `LIGHTNING_MAX_STRIKES` (default 5000, at least 1): buffer cap.
    pub max_strikes: usize,
    /// Primary endpoints (replaced by `LIGHTNING_WS_ENDPOINTS` when set).
    pub primary: Vec<String>,
    /// Legacy endpoints. Never overridden.
    pub legacy: Vec<String>,
    /// Accepted strike area.
    pub bbox: BoundingBox,
    /// Heartbeat and flush cadence. 30 s in production. Tests shorten it.
    pub heartbeat_every: Duration,
}

impl LightningConfig {
    /// Defaults for `state_dir`: spec 6.2 values, default endpoint lists.
    #[must_use]
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            retention_min: 15,
            flush_every_s: 2.0,
            max_strikes: 5000,
            primary: primary_endpoints(),
            legacy: legacy_endpoints(),
            bbox: BoundingBox::default(),
            heartbeat_every: Duration::from_secs(30),
        }
    }

    /// Read every knob from the environment. Unset or empty variables use the defaults. A set but invalid value
    /// fails with [`EnvError`] (radar-core strict parsing).
    pub fn from_env() -> Result<Self, EnvError> {
        let state_dir = path_var("STATE_DIR", "/data/state");
        let mut cfg = Self::new(state_dir);
        cfg.retention_min = int_var("LIGHTNING_RETENTION_MIN", 15, 0, i64::MAX)?;
        cfg.flush_every_s = float_var("LIGHTNING_FLUSH_S", 2.0)?;
        cfg.max_strikes = int_var("LIGHTNING_MAX_STRIKES", 5000, 1, i64::MAX)? as usize;
        if let Some(raw) = var("LIGHTNING_WS_ENDPOINTS") {
            let override_list = parse_endpoint_override(&raw);
            if !override_list.is_empty() {
                cfg.primary = override_list;
            }
        }
        cfg.bbox = BoundingBox {
            lat_min: float_var("LIGHTNING_LAT_MIN", 15.0)?,
            lat_max: float_var("LIGHTNING_LAT_MAX", 55.0)?,
            lon_min: float_var("LIGHTNING_LON_MIN", -130.0)?,
            lon_max: float_var("LIGHTNING_LON_MAX", -50.0)?,
        };
        Ok(cfg)
    }

    /// Path of the published GeoJSON file.
    #[must_use]
    pub fn out_path(&self) -> PathBuf {
        self.state_dir.join(OUT_FILE)
    }

    /// State directory as a path reference.
    #[must_use]
    pub fn state(&self) -> &Path {
        &self.state_dir
    }
}

/// File name of the published strike layer, inside `STATE_DIR`.
pub const OUT_FILE: &str = "lightning.json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_and_legacy_lists_match_spec_6_1() {
        let primary = primary_endpoints();
        assert_eq!(
            primary,
            vec![
                "wss://ws1.blitzortung.org:443/",
                "wss://ws2.blitzortung.org:443/",
                "wss://ws7.blitzortung.org:443/",
                "wss://ws8.blitzortung.org:443/",
            ]
        );
        let legacy = legacy_endpoints();
        assert_eq!(legacy.len(), 32);
        assert_eq!(legacy[0], "ws://ws1.blitzortung.org:8087/");
        assert_eq!(legacy[1], "ws://ws1.blitzortung.org:8088/");
        assert_eq!(legacy[31], "ws://ws8.blitzortung.org:8090/");
        assert!(legacy.iter().all(|u| u.starts_with("ws://")));
        assert!(
            primary.iter().all(|u| !legacy.contains(u)),
            "lists are disjoint"
        );
    }

    #[test]
    fn override_is_trimmed_and_drops_empties() {
        assert_eq!(
            parse_endpoint_override(" wss://relay.local/ ,, ws://other.local:8087/ "),
            vec!["wss://relay.local/", "ws://other.local:8087/"]
        );
        assert!(parse_endpoint_override(" , ,").is_empty());
    }

    #[test]
    fn bbox_is_inclusive_and_rejects_nan() {
        let b = BoundingBox::default();
        assert!(b.contains(15.0, -130.0));
        assert!(b.contains(55.0, -50.0));
        assert!(!b.contains(55.0001, -60.0));
        assert!(!b.contains(f64::NAN, -60.0));
    }
}
