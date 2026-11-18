//! The 1991-2020 daily temperature normals store (design `weather-extras.md` section 6).
//!
//! `STATE_DIR/static/normals/` holds `tmax.i16` and `tmin.i16`: 366 days × rows × cols, little-endian
//! int16 tenths of a degree C, north-south rows from the southern edge, [`MISSING`] off land. Day
//! index 59 is Feb 29. `normals.json` is written last and is the marker that the store is complete.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use super::hdf5::{self, Layout, ReadAt};

/// Store directory under `STATE_DIR`.
pub const NORMALS_DIR: &str = "static/normals";
/// The marker and grid description, written after both arrays.
pub const META_FILE: &str = "normals.json";
/// Days in the files, Feb 29 included.
pub const DAYS: usize = 366;
/// Stored value of a cell with no normal (ocean, outside the lower 48).
pub const MISSING: i16 = i16::MIN;
/// Store format version.
pub const FORMAT_VERSION: u32 = 1;

/// NCEI archive directory of the nClimGrid-Daily normals.
pub const SOURCE_BASE_URL: &str =
    "https://www.ncei.noaa.gov/data/oceans/archive/arc0207/0259962/1.1/data/0-data";

/// One of the two normals arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Var {
    /// Normal daily high.
    Tmax,
    /// Normal daily low.
    Tmin,
}

impl Var {
    /// Both arrays, in download order.
    pub const ALL: [Var; 2] = [Var::Tmax, Var::Tmin];

    /// Short name, also the stored file stem.
    pub fn name(self) -> &'static str {
        match self {
            Var::Tmax => "tmax",
            Var::Tmin => "tmin",
        }
    }

    /// Stored file name.
    pub fn file(self) -> String {
        format!("{}.i16", self.name())
    }

    /// The NetCDF variable holding the daily normal.
    pub fn nc_variable(self) -> &'static str {
        match self {
            Var::Tmax => "dlytmax_norm",
            Var::Tmin => "dlytmin_norm",
        }
    }

    /// Source URL of the NetCDF file.
    pub fn source_url(self, base: &str) -> String {
        format!(
            "{}/{}-1991_2020-daily-normals-v1.0.nc",
            base.trim_end_matches('/'),
            self.name()
        )
    }
}

/// The regular lat/lon grid of the store.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GridSpec {
    /// Rows (latitudes), south first.
    pub rows: usize,
    /// Columns (longitudes), west first.
    pub cols: usize,
    /// Centre of the first row, degrees.
    pub lat0: f64,
    /// Centre of the first column, degrees.
    pub lon0: f64,
    /// Cell size, degrees (1/24).
    pub step: f64,
}

impl GridSpec {
    /// Bytes of one day plane.
    pub fn plane_bytes(&self) -> u64 {
        (self.rows * self.cols * 2) as u64
    }

    /// Bytes of one stored array.
    pub fn file_bytes(&self) -> u64 {
        self.plane_bytes() * DAYS as u64
    }

    /// Nearest `(row, col)`, or `None` outside the grid.
    pub fn cell(&self, lat: f64, lon: f64) -> Option<(usize, usize)> {
        let r = ((lat - self.lat0) / self.step).round();
        let c = ((lon - self.lon0) / self.step).round();
        (r >= 0.0 && c >= 0.0 && (r as usize) < self.rows && (c as usize) < self.cols)
            .then_some((r as usize, c as usize))
    }
}

/// `normals.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalsMeta {
    /// [`FORMAT_VERSION`].
    pub version: u32,
    /// The grid of both arrays.
    pub grid: GridSpec,
    /// Days per array ([`DAYS`]).
    pub days: usize,
    /// Degrees C per stored unit (0.1).
    pub scale: f64,
    /// Climate period.
    pub period: String,
    /// Source URLs.
    pub sources: Vec<String>,
    /// Bytes of both arrays on disk.
    pub bytes: u64,
    /// When the store was built (RFC 3339).
    pub created_at: String,
}

/// File index of a calendar date. Day 60 of the file (index 59) is Feb 29 and is used only on Feb 29.
pub fn day_index(date: NaiveDate) -> usize {
    NaiveDate::from_ymd_opt(2000, date.month(), date.day())
        .expect("every month/day exists in a leap year")
        .ordinal0() as usize
}

/// Store directory for a state directory.
pub fn store_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(NORMALS_DIR)
}

/// The normal high and low of one cell and day, degrees C.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayNormal {
    /// Normal daily high.
    pub high_c: f64,
    /// Normal daily low.
    pub low_c: f64,
}

/// An opened, complete normals store.
#[derive(Debug, Clone)]
pub struct Normals {
    dir: PathBuf,
    /// The marker contents.
    pub meta: NormalsMeta,
}

fn decode(v: i16) -> Option<f64> {
    (v != MISSING).then(|| f64::from(v) / 10.0)
}

impl Normals {
    /// The store under `state_dir`, or `None` until the marker exists.
    pub fn open(state_dir: &Path) -> io::Result<Option<Normals>> {
        let dir = store_dir(state_dir);
        let bytes = match fs::read(dir.join(META_FILE)) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let meta: NormalsMeta = serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if meta.version != FORMAT_VERSION || meta.days != DAYS {
            return Ok(None);
        }
        Ok(Some(Normals { dir, meta }))
    }

    fn read_cells(&self, var: Var, day: usize, cells: &[(usize, usize)]) -> io::Result<Vec<i16>> {
        use std::os::unix::fs::FileExt;
        let f = File::open(self.dir.join(var.file()))?;
        let g = &self.meta.grid;
        cells
            .iter()
            .map(|&(r, c)| {
                let mut b = [0u8; 2];
                let off = day as u64 * g.plane_bytes() + ((r * g.cols + c) * 2) as u64;
                f.read_exact_at(&mut b, off)?;
                Ok(i16::from_le_bytes(b))
            })
            .collect()
    }

    /// The normal high and low at the nearest land cell (within one cell) on `date`, or `None` off
    /// the grid or over water.
    pub fn point(&self, lat: f64, lon: f64, date: NaiveDate) -> io::Result<Option<DayNormal>> {
        let g = self.meta.grid;
        let Some((r0, c0)) = g.cell(lat, lon) else {
            return Ok(None);
        };
        // The centre first, then its neighbours nearest first: coastal points keep a value.
        let mut cells = vec![(r0, c0)];
        for (dr, dc) in [
            (-1i64, 0i64),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ] {
            let (r, c) = (r0 as i64 + dr, c0 as i64 + dc);
            if r >= 0 && c >= 0 && (r as usize) < g.rows && (c as usize) < g.cols {
                cells.push((r as usize, c as usize));
            }
        }
        let day = day_index(date);
        let highs = self.read_cells(Var::Tmax, day, &cells)?;
        let lows = self.read_cells(Var::Tmin, day, &cells)?;
        Ok(highs.iter().zip(&lows).find_map(|(&h, &l)| {
            Some(DayNormal {
                high_c: decode(h)?,
                low_c: decode(l)?,
            })
        }))
    }

    /// One day plane of `var`, row-major, south first.
    pub fn plane(&self, var: Var, date: NaiveDate) -> io::Result<Vec<i16>> {
        use std::os::unix::fs::FileExt;
        let g = &self.meta.grid;
        let f = File::open(self.dir.join(var.file()))?;
        let mut buf = vec![0u8; g.plane_bytes() as usize];
        f.read_exact_at(&mut buf, day_index(date) as u64 * g.plane_bytes())?;
        Ok(buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect())
    }

    /// The value of `plane` at the nearest cell, degrees C.
    pub fn sample(&self, plane: &[i16], lat: f64, lon: f64) -> Option<f64> {
        let g = &self.meta.grid;
        let (r, c) = g.cell(lat, lon)?;
        decode(*plane.get(r * g.cols + c)?)
    }
}

/// tenths of a degree C, or [`MISSING`] for NaN and fill values.
pub fn to_tenths(v: f32) -> i16 {
    if !v.is_finite() || v.abs() > 100.0 {
        return MISSING;
    }
    (f64::from(v) * 10.0).round() as i16
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Read the grid axes and check they are regular. Returns the grid of the file.
pub fn read_grid(r: &mut dyn ReadAt) -> io::Result<GridSpec> {
    let lat = hdf5::dataset(r, "lat")?;
    let lat = hdf5::read_f32_vector(r, &lat)?;
    let lon = hdf5::dataset(r, "lon")?;
    let lon = hdf5::read_f32_vector(r, &lon)?;
    if lat.len() < 2 || lon.len() < 2 {
        return Err(invalid("axes too short"));
    }
    let step = f64::from(lat[1]) - f64::from(lat[0]);
    let regular = |axis: &[f32]| {
        axis.iter()
            .enumerate()
            .all(|(i, &v)| (f64::from(v) - (f64::from(axis[0]) + step * i as f64)).abs() < 1e-3)
    };
    if step <= 0.0 || !regular(&lat) || !regular(&lon) {
        return Err(invalid("axes are not a regular ascending grid"));
    }
    Ok(GridSpec {
        rows: lat.len(),
        cols: lon.len(),
        lat0: f64::from(lat[0]),
        lon0: f64::from(lon[0]),
        step: 1.0 / (1.0 / step).round(),
    })
}

/// The 366 day chunks of `var`, after checking the dataset is the uncompressed one-chunk-per-day
/// float array the store expects.
pub fn day_chunks(r: &mut dyn ReadAt, var: Var, grid: &GridSpec) -> io::Result<Vec<hdf5::Chunk>> {
    let ds = hdf5::dataset(r, var.nc_variable())?;
    let plane = (grid.rows * grid.cols) as u64;
    if ds.dims != [DAYS as u64, grid.rows as u64, grid.cols as u64] {
        return Err(invalid(format!(
            "{} has dims {:?}",
            var.nc_variable(),
            ds.dims
        )));
    }
    if !ds.float_le || ds.element_size != 4 || ds.filtered {
        return Err(invalid(format!("{} is not raw float32", var.nc_variable())));
    }
    let Layout::Chunked { btree, chunk } = &ds.layout else {
        return Err(invalid("normals are not chunked by day"));
    };
    if chunk.as_slice() != [1, grid.rows as u64, grid.cols as u64] {
        return Err(invalid(format!("chunk shape {chunk:?}")));
    }
    let chunks = hdf5::chunks(r, *btree)?;
    let ok = chunks.len() == DAYS
        && chunks.iter().enumerate().all(|(d, c)| {
            c.offset == [d as u64, 0, 0] && u64::from(c.size) == plane * 4 && c.filter_mask == 0
        });
    if !ok {
        return Err(invalid("chunk index is not one raw chunk per day"));
    }
    Ok(chunks)
}

/// Convert one NetCDF array to `out` (int16 tenths, temp file then rename). `progress` gets the day.
pub fn convert(
    r: &mut dyn ReadAt,
    var: Var,
    grid: &GridSpec,
    out: &Path,
    progress: &mut dyn FnMut(usize),
) -> io::Result<u64> {
    let chunks = day_chunks(r, var, grid)?;
    let dir = out.parent().ok_or_else(|| invalid("no parent directory"))?;
    fs::create_dir_all(dir)?;
    let tmp = tempfile::Builder::new()
        .prefix(&format!(".{}.", var.name()))
        .suffix(".tmp")
        .tempfile_in(dir)?;
    let mut w = BufWriter::with_capacity(1 << 20, tmp.as_file());
    for (d, c) in chunks.iter().enumerate() {
        let raw = r.read_at(c.address, c.size as usize)?;
        let mut plane = Vec::with_capacity(raw.len() / 2);
        for b in raw.as_chunks::<4>().0 {
            let v = f32::from_le_bytes(*b);
            plane.extend_from_slice(&to_tenths(v).to_le_bytes());
        }
        w.write_all(&plane)?;
        progress(d);
    }
    w.flush()?;
    drop(w);
    tmp.as_file().sync_all()?;
    tmp.persist(out).map_err(|e| e.error)?;
    radar_core::fsutil::fsync_dir(dir)?;
    Ok(fs::metadata(out)?.len())
}

/// Write the marker, completing the store.
pub fn write_meta(state_dir: &Path, meta: &NormalsMeta) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(meta).map_err(|e| invalid(e.to_string()))?;
    radar_core::fsutil::atomic_write(
        &store_dir(state_dir).join(META_FILE),
        &bytes,
        ".normals.",
        ".tmp",
    )
}

/// Write a complete store from a value function `(var, day, row, col) -> tenths C`. For tests and
/// fixtures; the real store comes from [`convert`].
pub fn write_store(
    state_dir: &Path,
    grid: GridSpec,
    value: impl Fn(Var, usize, usize, usize) -> i16,
) -> io::Result<NormalsMeta> {
    let dir = store_dir(state_dir);
    fs::create_dir_all(&dir)?;
    for var in Var::ALL {
        let mut w = BufWriter::new(File::create(dir.join(var.file()))?);
        for day in 0..DAYS {
            for r in 0..grid.rows {
                for c in 0..grid.cols {
                    w.write_all(&value(var, day, r, c).to_le_bytes())?;
                }
            }
        }
        w.flush()?;
    }
    let meta = NormalsMeta {
        version: FORMAT_VERSION,
        grid,
        days: DAYS,
        scale: 0.1,
        period: "1991-2020".into(),
        sources: Vec::new(),
        bytes: 2 * grid.file_bytes(),
        created_at: "2026-01-01T00:00:00+00:00".into(),
    };
    write_meta(state_dir, &meta)?;
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_index_uses_feb_29_only_on_feb_29() {
        let d = |y, m, day| day_index(NaiveDate::from_ymd_opt(y, m, day).unwrap());
        assert_eq!(d(2026, 1, 1), 0);
        assert_eq!(d(2026, 2, 28), 58);
        assert_eq!(d(2028, 2, 29), 59);
        // A common year skips the Feb 29 slot.
        assert_eq!(d(2026, 3, 1), 60);
        assert_eq!(d(2028, 3, 1), 60);
        assert_eq!(d(2026, 12, 31), 365);
    }

    #[test]
    fn tenths_round_and_mark_fill_values_missing() {
        assert_eq!(to_tenths(0.86), 9);
        assert_eq!(to_tenths(-12.34), -123);
        assert_eq!(to_tenths(f32::NAN), MISSING);
        assert_eq!(to_tenths(-9999.0), MISSING);
    }

    #[test]
    fn cell_is_nearest_and_none_outside() {
        let g = GridSpec {
            rows: 596,
            cols: 1385,
            lat0: 24.5625,
            lon0: -124.6875,
            step: 1.0 / 24.0,
        };
        assert_eq!(g.cell(42.2125, -83.3534), Some((424, 992)));
        assert_eq!(g.cell(24.5625, -124.6875), Some((0, 0)));
        assert_eq!(g.cell(21.3, -157.8), None);
        assert_eq!(g.cell(61.2, -149.9), None);
    }
}
