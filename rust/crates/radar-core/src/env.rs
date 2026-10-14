//! Typed environment readers and the on-disk [`Paths`] layout.
//!
//! Names and defaults follow spec §0.5. An unset or empty variable means "use the default"
//! (Kubernetes often sets empty strings). A set but unparseable value is an error, so the process
//! fails at startup instead of silently running with a surprising value. Python would raise at import.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// A set environment variable that cannot be parsed.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid value for {name}: {value:?} ({reason})")]
pub struct EnvError {
    /// Variable name.
    pub name: String,
    /// Raw value.
    pub value: String,
    /// Why it was rejected.
    pub reason: &'static str,
}

/// The variable's value, or `None` when unset or empty.
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Parse a boolean flag. Accepts `1`, `true`, `yes`, `on` (any case) as true and `0`, `false`, `no`, `off`
/// as false. Anything else is rejected. (PLAN temporal D5: the Python `PUSH_DISABLED` check was truthy
/// for everything except `"1"`.)
pub fn parse_bool(name: &str, value: &str) -> Result<bool, EnvError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(EnvError {
            name: name.to_string(),
            value: value.to_string(),
            reason: "expected one of 1/0, true/false, yes/no, on/off",
        }),
    }
}

/// Boolean variable with a default.
pub fn bool_var(name: &str, default: bool) -> Result<bool, EnvError> {
    bool_from(name, var(name), default)
}

/// [`bool_var`] on an already-read raw value.
pub fn bool_from(name: &str, raw: Option<String>, default: bool) -> Result<bool, EnvError> {
    match raw.filter(|v| !v.is_empty()) {
        None => Ok(default),
        Some(v) => parse_bool(name, &v),
    }
}

/// Integer variable with a default. Out-of-range values are an error.
pub fn int_var(name: &str, default: i64, min: i64, max: i64) -> Result<i64, EnvError> {
    int_from(name, var(name), default, min, max)
}

/// [`int_var`] on an already-read raw value (`None` = unset). Split out so tests need no env mutation.
pub fn int_from(
    name: &str,
    raw: Option<String>,
    default: i64,
    min: i64,
    max: i64,
) -> Result<i64, EnvError> {
    let Some(v) = raw.filter(|v| !v.is_empty()) else {
        return Ok(default);
    };
    let parsed = v.trim().parse::<i64>().map_err(|_| EnvError {
        name: name.to_string(),
        value: v.clone(),
        reason: "not an integer",
    })?;
    if parsed < min || parsed > max {
        return Err(EnvError {
            name: name.to_string(),
            value: v,
            reason: "out of range",
        });
    }
    Ok(parsed)
}

/// Integer variable with a default, clamped into `[min, max]` (the Python `max(0, int(...))` style).
pub fn int_var_clamped(name: &str, default: i64, min: i64, max: i64) -> Result<i64, EnvError> {
    int_clamped_from(name, var(name), default, min, max)
}

/// [`int_var_clamped`] on an already-read raw value.
pub fn int_clamped_from(
    name: &str,
    raw: Option<String>,
    default: i64,
    min: i64,
    max: i64,
) -> Result<i64, EnvError> {
    let Some(v) = raw.filter(|v| !v.is_empty()) else {
        return Ok(default);
    };
    let parsed = v.trim().parse::<i64>().map_err(|_| EnvError {
        name: name.to_string(),
        value: v,
        reason: "not an integer",
    })?;
    Ok(parsed.clamp(min, max))
}

/// Finite float variable with a default.
pub fn float_var(name: &str, default: f64) -> Result<f64, EnvError> {
    float_from(name, var(name), default)
}

/// [`float_var`] on an already-read raw value.
pub fn float_from(name: &str, raw: Option<String>, default: f64) -> Result<f64, EnvError> {
    let Some(v) = raw.filter(|v| !v.is_empty()) else {
        return Ok(default);
    };
    v.trim()
        .parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
        .ok_or_else(|| EnvError {
            name: name.to_string(),
            value: v,
            reason: "not a finite number",
        })
}

/// Path variable with a default.
pub fn path_var(name: &str, default: &str) -> PathBuf {
    PathBuf::from(var(name).unwrap_or_else(|| default.to_string()))
}

/// Root directories of the backend data plane (spec §2). Defaults: `/data/state`, `/data/grids`, `/data/tiles`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `STATE_DIR`: manifest, storms, nowcast skill log, ProcessedSets, push tokens.
    pub state_dir: PathBuf,
    /// `GRID_DIR`: per-layer grid dumps.
    pub grid_dir: PathBuf,
    /// `TILE_DIR`: rendered tile pyramid (existence checks only in this crate).
    pub tile_dir: PathBuf,
}

impl Paths {
    /// Resolve from `STATE_DIR`, `GRID_DIR` and `TILE_DIR`, with their spec defaults.
    pub fn from_env() -> Self {
        Self {
            state_dir: path_var("STATE_DIR", "/data/state"),
            grid_dir: path_var("GRID_DIR", "/data/grids"),
            tile_dir: path_var("TILE_DIR", "/data/tiles"),
        }
    }

    /// Explicit roots (tests and tools).
    pub fn new(
        state_dir: impl Into<PathBuf>,
        grid_dir: impl Into<PathBuf>,
        tile_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            state_dir: state_dir.into(),
            grid_dir: grid_dir.into(),
            tile_dir: tile_dir.into(),
        }
    }

    /// `<STATE_DIR>/manifest.json`.
    pub fn manifest_path(&self) -> PathBuf {
        self.state_dir.join(crate::manifest::MANIFEST_FILE)
    }

    /// `<STATE_DIR>/nowcast-skill.json`.
    pub fn nowcast_skill_path(&self) -> PathBuf {
        self.state_dir.join(crate::nowcast_skill::SKILL_FILE)
    }

    /// `<STATE_DIR>/push_tokens.sqlite`.
    pub fn push_tokens_db(&self) -> PathBuf {
        self.state_dir.join(crate::push_tokens::DB_FILE)
    }

    /// A named file in the state directory (ProcessedSet files such as `ingest-hrrr.json`).
    pub fn state_file(&self, name: &str) -> PathBuf {
        self.state_dir.join(name)
    }

    /// `<GRID_DIR>/<layer>`.
    pub fn grid_layer_dir(&self, layer: &str) -> PathBuf {
        self.grid_dir.join(layer)
    }

    /// `<TILE_DIR>/<layer>`.
    pub fn tile_layer_dir(&self, layer: &str) -> PathBuf {
        self.tile_dir.join(layer)
    }

    /// Borrow the state directory.
    pub fn state(&self) -> &Path {
        &self.state_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_parsing_accepts_documented_spellings() {
        for v in ["1", "true", "TRUE", "yes", "On", " on "] {
            assert_eq!(parse_bool("X", v), Ok(true), "{v}");
        }
        for v in ["0", "false", "No", "OFF"] {
            assert_eq!(parse_bool("X", v), Ok(false), "{v}");
        }
        assert!(parse_bool("X", "maybe").is_err());
        assert!(parse_bool("X", "").is_err());
    }

    #[test]
    fn int_readers_default_clamp_and_reject() {
        assert_eq!(
            int_clamped_from("X", Some(String::new()), 300, 0, i64::MAX),
            Ok(300)
        );
        assert_eq!(
            int_clamped_from("X", Some("-5".into()), 300, 0, i64::MAX),
            Ok(0)
        );
        assert!(int_clamped_from("X", Some("7x".into()), 300, 0, i64::MAX).is_err());
        assert!(int_from("X", Some("-5".into()), 1, 0, 10).is_err());
        assert_eq!(int_from("X", None, 9, 0, 10), Ok(9));
        assert!(float_from("X", Some("NaN".into()), 1.0).is_err());
        assert_eq!(bool_from("X", Some("yes".into()), false), Ok(true));
    }

    #[test]
    fn paths_resolve_relative_layout() {
        let p = Paths::new("/s", "/g", "/t");
        assert_eq!(p.manifest_path(), PathBuf::from("/s/manifest.json"));
        assert_eq!(p.push_tokens_db(), PathBuf::from("/s/push_tokens.sqlite"));
        assert_eq!(p.grid_layer_dir("radar"), PathBuf::from("/g/radar"));
        assert_eq!(p.tile_layer_dir("radar"), PathBuf::from("/t/radar"));
        assert_eq!(
            p.state_file("ingest-hrrr.json"),
            PathBuf::from("/s/ingest-hrrr.json")
        );
    }
}
