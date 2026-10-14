//! Grid readers: parse `<key>.meta.json`, read the immutable `.bin`, and bilinear-sample a point.
//!
//! Readers take no lock. They rely on the atomic metadata pointer (spec-shared §5.9). The sampler follows
//! spec-api §9.2 (`_sample_grid_point`) operation for operation, including the top-corner fill quirk, so the
//! inspector and nowcast output match Python bit-for-bit. Known deviations are listed in README.md.

use crate::grid_dump;
use crate::pycompat::py_int;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors from reading a grid.
#[derive(Debug, Error)]
pub enum GridReadError {
    /// The metadata file does not exist.
    #[error("metadata missing: {0}")]
    MetaMissing(PathBuf),
    /// The metadata is unreadable, not JSON, or lacks a required field.
    #[error("metadata invalid: {0}")]
    MetaInvalid(String),
    /// The data file the metadata points to does not exist.
    #[error("data file missing: {0}")]
    DataMissing(PathBuf),
    /// The data file is shorter than `height * width * 4` bytes.
    #[error("data file too short: expected {expected} bytes, found {actual}")]
    ShortRead {
        /// Bytes required by the metadata.
        expected: usize,
        /// Bytes present.
        actual: usize,
    },
    /// Some other I/O failure.
    #[error("grid I/O: {0}")]
    Io(#[from] std::io::Error),
    /// The grid cannot be sampled: fewer than 2 rows or columns, or a zero-width extent.
    #[error("grid too small or degenerate to sample")]
    Degenerate,
}

/// Parsed `<key>.meta.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct GridMeta {
    /// Data file name (sibling of the meta).
    pub data_file: String,
    /// Rows.
    pub height: usize,
    /// Columns.
    pub width: usize,
    /// Southern edge, degrees.
    pub lat_min: f64,
    /// Northern edge, degrees.
    pub lat_max: f64,
    /// Western edge, degrees.
    pub lon_min: f64,
    /// Eastern edge, degrees.
    pub lon_max: f64,
    /// Unit label (empty when absent).
    pub unit: String,
    /// No-data sentinel as stored. NaN when absent.
    pub fill: f64,
    /// Decimation factor used when writing.
    pub stride: i64,
}

/// A decoded grid: metadata plus `height * width` values, row 0 north, column 0 west.
#[derive(Debug, Clone, PartialEq)]
pub struct GridDump {
    /// The metadata.
    pub meta: GridMeta,
    /// Row-major little-endian f32 values as stored (no-data cells keep the sentinel).
    pub values: Vec<f32>,
}

/// Result of [`GridDump::sample`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sample {
    /// The point is outside the grid's extent (NaN coordinates also land here).
    OutOfBounds,
    /// Every neighbour is no-data.
    NoData,
    /// Interpolated value.
    Value(f64),
}

/// Read and parse a metadata file.
pub fn read_meta(meta_path: &Path) -> Result<GridMeta, GridReadError> {
    let text = match fs::read_to_string(meta_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(GridReadError::MetaMissing(meta_path.to_path_buf()));
        }
        Err(e) => return Err(GridReadError::Io(e)),
    };
    let doc: Value =
        serde_json::from_str(&text).map_err(|e| GridReadError::MetaInvalid(e.to_string()))?;
    let Value::Object(obj) = doc else {
        return Err(GridReadError::MetaInvalid(
            "metadata is not an object".into(),
        ));
    };
    let int_field = |key: &str| -> Result<i64, GridReadError> {
        obj.get(key)
            .and_then(py_int)
            .ok_or_else(|| GridReadError::MetaInvalid(format!("missing or non-integer {key}")))
    };
    let float_field = |key: &str| -> Result<f64, GridReadError> {
        obj.get(key)
            .and_then(as_python_float)
            .ok_or_else(|| GridReadError::MetaInvalid(format!("missing or non-numeric {key}")))
    };
    let height = usize::try_from(int_field("height")?)
        .map_err(|_| GridReadError::MetaInvalid("negative height".into()))?;
    let width = usize::try_from(int_field("width")?)
        .map_err(|_| GridReadError::MetaInvalid("negative width".into()))?;
    let data_file = match obj.get("data_file") {
        None | Some(Value::Null) => legacy_data_file(meta_path),
        Some(Value::String(s)) if is_plain(s) => s.clone(),
        Some(_) => {
            return Err(GridReadError::MetaInvalid(
                "data_file is not a plain file name".into(),
            ));
        }
    };
    let unit = match obj.get("unit") {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    let fill = obj
        .get("fill")
        .and_then(as_python_float)
        .unwrap_or(f64::NAN);
    let stride = obj.get("stride").and_then(py_int).unwrap_or(1);
    Ok(GridMeta {
        data_file,
        height,
        width,
        lat_min: float_field("lat_min")?,
        lat_max: float_field("lat_max")?,
        lon_min: float_field("lon_min")?,
        lon_max: float_field("lon_max")?,
        unit,
        fill,
        stride,
    })
}

/// Read the metadata and its data file.
pub fn read_grid(meta_path: &Path) -> Result<GridDump, GridReadError> {
    let meta = read_meta(meta_path)?;
    let bin_path = meta_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&meta.data_file);
    let bytes = match fs::read(&bin_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(GridReadError::DataMissing(bin_path));
        }
        Err(e) => return Err(GridReadError::Io(e)),
    };
    let count = meta.height * meta.width;
    let expected = count * 4;
    if bytes.len() < expected {
        return Err(GridReadError::ShortRead {
            expected,
            actual: bytes.len(),
        });
    }
    let values = bytes[..expected]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    Ok(GridDump { meta, values })
}

impl GridDump {
    /// Value at `(row, col)`, row 0 north. Panics when out of range, like indexing.
    pub fn at(&self, row: usize, col: usize) -> f32 {
        self.values[row * self.meta.width + col]
    }

    /// Bilinear sample at `(lat, lon)`, operation-for-operation with spec-api §9.2:
    /// - out-of-extent points (and NaN) give [`Sample::OutOfBounds`];
    /// - a fill neighbour is excluded. When a top corner is filled, `v0` takes `valid[0]` and `v1` takes
    ///   `valid[-1]` (the quirk the spec verified: NW fill at (42.5, -85.5) gives 27.5);
    /// - when all four are fill, [`Sample::NoData`].
    pub fn sample(&self, lat: f64, lon: f64) -> Result<Sample, GridReadError> {
        let m = &self.meta;
        if m.height < 2 || m.width < 2 {
            return Err(GridReadError::Degenerate);
        }
        if lat.is_nan() || lon.is_nan() {
            return Ok(Sample::OutOfBounds);
        }
        if lat < m.lat_min || lat > m.lat_max || lon < m.lon_min || lon > m.lon_max {
            return Ok(Sample::OutOfBounds);
        }
        let lon_span = m.lon_max - m.lon_min;
        let lat_span = m.lat_max - m.lat_min;
        if lon_span == 0.0 || lat_span == 0.0 {
            return Err(GridReadError::Degenerate);
        }
        let (h, w) = (m.height, m.width);
        let fx = (lon - m.lon_min) / lon_span * (w - 1) as f64;
        let fy = (m.lat_max - lat) / lat_span * (h - 1) as f64;
        let x0 = (fx.trunc() as i64).clamp(0, w as i64 - 2) as usize;
        let y0 = (fy.trunc() as i64).clamp(0, h as i64 - 2) as usize;
        let dx = fx - x0 as f64;
        let dy = fy - y0 as f64;
        let v00 = f64::from(self.at(y0, x0));
        let v10 = f64::from(self.at(y0, x0 + 1));
        let v01 = f64::from(self.at(y0 + 1, x0));
        let v11 = f64::from(self.at(y0 + 1, x0 + 1));
        let fill = m.fill;
        let is_fill = |v: f64| v.is_nan() || (v - fill).abs() < 1e-6;
        let valid: Vec<f64> = [v00, v10, v01, v11]
            .into_iter()
            .filter(|v| !is_fill(*v))
            .collect();
        if valid.is_empty() {
            return Ok(Sample::NoData);
        }
        let v0 = if !is_fill(v00) && !is_fill(v10) {
            v00 * (1.0 - dx) + v10 * dx
        } else {
            valid[0]
        };
        let v1 = if !is_fill(v01) && !is_fill(v11) {
            v01 * (1.0 - dx) + v11 * dx
        } else {
            valid[valid.len() - 1]
        };
        Ok(Sample::Value(v0 * (1.0 - dy) + v1 * dy))
    }
}

/// Python `float(value)` for the fields the readers take: numbers and numeric strings.
fn as_python_float(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn is_plain(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/')
}

/// Legacy data file name: `<stem>.bin` for `<stem>.meta.json`.
fn legacy_data_file(meta_path: &Path) -> String {
    let name = meta_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    format!("{}.bin", grid_dump::grid_timestamp_of(name))
}

#[cfg(test)]
mod tests;
