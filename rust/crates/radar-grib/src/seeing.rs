//! The star-seeing store: ECCC RDPS seeing and sky transparency (0-5) for one valid hour.
//!
//! One file per valid hour, `star-seeing/<YYYYMMDDTHHZ>.bin` under `STATE_DIR`: an 80-byte header
//! (magic, grid, model run) then `nj * ni` cells in GRIB scan order, one byte each:
//! `transparency << 4 | seeing`, [`MISSING`] when either is absent. The aux `star-seeing` product
//! writes it; the HRRR stargazing render reads it whole and the API reads one cell.
//!
//! ECCC's scale, from the GeoMet legends: 0 zero (cloudy sky), 1 very bad, 2 poor,
//! 3 average, 4 good, 5 excellent.

use chrono::{DateTime, Utc};

use crate::rotated::{RotatedField, RotatedGrid, RotatedPole};

/// Directory under `STATE_DIR`.
pub const SEEING_DIR: &str = "star-seeing";
/// Header length in bytes.
pub const HEADER_LEN: usize = 80;
/// A cell with no value.
pub const MISSING: u8 = 0xFF;
const MAGIC: &[u8; 8] = b"RNSEE01\n";

/// File name of a valid hour, `YYYYMMDDTHHZ.bin`.
#[must_use]
pub fn file_name(valid: DateTime<Utc>) -> String {
    valid.format("%Y%m%dT%HZ.bin").to_string()
}

/// Seeing and transparency for one valid hour.
#[derive(Debug, Clone, PartialEq)]
pub struct SeeingHour {
    /// Grid of both fields.
    pub grid: RotatedGrid,
    /// Model run, Unix seconds.
    pub run_unix: i64,
    /// `nj * ni` packed cells.
    pub cells: Vec<u8>,
}

/// One packed cell: `(seeing, transparency)`, each 0-5, or `None` when missing.
#[inline]
#[must_use]
pub fn unpack(cell: u8) -> Option<(u8, u8)> {
    (cell != MISSING).then_some((cell & 0x0F, cell >> 4))
}

fn class(v: f32) -> Option<u8> {
    (v.is_finite() && (0.0..=5.0).contains(&v)).then(|| v.round() as u8)
}

impl SeeingHour {
    /// Packs two decoded fields on the same grid. `None` when the grids differ.
    #[must_use]
    pub fn from_fields(
        seeing: &RotatedField,
        transparency: &RotatedField,
        run_unix: i64,
    ) -> Option<Self> {
        if seeing.grid != transparency.grid {
            return None;
        }
        let cells = seeing
            .values
            .iter()
            .zip(&transparency.values)
            .map(|(&s, &t)| match (class(s), class(t)) {
                (Some(s), Some(t)) => t << 4 | s,
                _ => MISSING,
            })
            .collect();
        Some(Self {
            grid: seeing.grid,
            run_unix,
            cells,
        })
    }

    /// The file bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let g = &self.grid;
        let mut out = Vec::with_capacity(HEADER_LEN + self.cells.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(g.ni as u32).to_le_bytes());
        out.extend_from_slice(&(g.nj as u32).to_le_bytes());
        for v in [
            g.lat0,
            g.lon0,
            g.dlat,
            g.dlon,
            g.pole.south_pole_lat,
            g.pole.south_pole_lon,
            g.pole.angle,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.run_unix.to_le_bytes());
        out.extend_from_slice(&self.cells);
        out
    }

    /// Parses file bytes. `None` when the header or length is wrong.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (grid, run_unix) = parse_header(bytes.get(..HEADER_LEN)?)?;
        let cells = bytes.get(HEADER_LEN..)?;
        (cells.len() == grid.ni * grid.nj).then(|| Self {
            grid,
            run_unix,
            cells: cells.to_vec(),
        })
    }

    /// `(seeing, transparency)` at the grid point nearest a geographic point.
    #[must_use]
    pub fn at(&self, lat: f64, lon: f64) -> Option<(u8, u8)> {
        let (r, c) = self.grid.nearest(lat, lon)?;
        unpack(self.cells[r * self.grid.ni + c])
    }

    /// [`score_factor`] at a point; 1 outside the grid or where a value is missing.
    #[must_use]
    pub fn factor_at(&self, lat: f64, lon: f64) -> f64 {
        self.at(lat, lon).map_or(1.0, |(s, t)| score_factor(s, t))
    }

    /// The stored hour `valid` under `state_dir`, if present and well formed.
    #[must_use]
    pub fn load(state_dir: &std::path::Path, valid: DateTime<Utc>) -> Option<Self> {
        let bytes = std::fs::read(state_dir.join(SEEING_DIR).join(file_name(valid))).ok()?;
        Self::decode(&bytes)
    }
}

/// Grid and run from the 80-byte header.
#[must_use]
pub fn parse_header(h: &[u8]) -> Option<(RotatedGrid, i64)> {
    if h.len() < HEADER_LEN || &h[..8] != MAGIC {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes(h[o..o + 4].try_into().unwrap_or_default()) as usize;
    let f = |o: usize| f64::from_le_bytes(h[o..o + 8].try_into().unwrap_or_default());
    let grid = RotatedGrid {
        ni: u(8),
        nj: u(12),
        lat0: f(16),
        lon0: f(24),
        dlat: f(32),
        dlon: f(40),
        pole: RotatedPole {
            south_pole_lat: f(48),
            south_pole_lon: f(56),
            angle: f(64),
        },
    };
    let run = i64::from_le_bytes(h[72..80].try_into().unwrap_or_default());
    let sane = grid.ni >= 2
        && grid.nj >= 2
        && grid.ni * grid.nj <= 4_000_000
        && [grid.lat0, grid.lon0, grid.dlat, grid.dlon]
            .iter()
            .all(|v| v.is_finite())
        && grid.dlon > 0.0
        && grid.dlat != 0.0;
    sane.then_some((grid, run))
}

/// Byte offset of the cell nearest a geographic point, for a one-byte read.
#[must_use]
pub fn cell_offset(grid: &RotatedGrid, lat: f64, lon: f64) -> Option<u64> {
    let (r, c) = grid.nearest(lat, lon)?;
    Some((HEADER_LEN + r * grid.ni + c) as u64)
}

/// Multiplier on the stargazing score. Our own weights: transparency hides faint stars, so it
/// counts more than seeing, which mostly blurs planets and doubles. Class 0 (cloudy) leaves the
/// score to the HRRR cloud cover, so the two models never double count cloud.
#[must_use]
pub fn score_factor(seeing: u8, transparency: u8) -> f64 {
    let t = match transparency {
        1 => 0.5,
        2 => 0.7,
        3 => 0.85,
        _ => 1.0,
    };
    let s = match seeing {
        1 => 0.85,
        2 => 0.9,
        3 => 0.95,
        _ => 1.0,
    };
    t * s
}

/// Plain word for a class, as ECCC's legend names it.
#[must_use]
pub fn class_word(v: u8) -> &'static str {
    match v {
        0 => "Cloudy",
        1 => "Very bad",
        2 => "Poor",
        3 => "Average",
        4 => "Good",
        _ => "Excellent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotated::decode_rotated;
    use chrono::TimeZone;

    fn fixture(name: &str) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(name);
        std::fs::read(p).unwrap()
    }

    fn hour() -> SeeingHour {
        let s = decode_rotated(&fixture("eccc_rdps_seeing_f006_greatlakes.grib2")).unwrap();
        let t = decode_rotated(&fixture("eccc_rdps_transparency_f006_greatlakes.grib2")).unwrap();
        SeeingHour::from_fields(&s, &t, 1_795_046_400).unwrap()
    }

    #[test]
    fn packs_round_trips_and_reads_a_city() {
        let h = hour();
        let bytes = h.encode();
        assert_eq!(bytes.len(), HEADER_LEN + 51 * 41);
        let back = SeeingHour::decode(&bytes).unwrap();
        assert_eq!(back, h);
        assert_eq!(back.at(43.65, -79.38), Some((3, 3)));
        assert_eq!(back.at(25.76, -80.19), None);
        let off = cell_offset(&back.grid, 43.65, -79.38).unwrap() as usize;
        assert_eq!(unpack(bytes[off]), Some((3, 3)));
        assert!(SeeingHour::decode(&bytes[..bytes.len() - 1]).is_none());
        assert!(SeeingHour::decode(b"not a seeing file").is_none());
    }

    #[test]
    fn missing_values_pack_as_missing() {
        let s = decode_rotated(&fixture("eccc_rdps_seeing_f006_greatlakes.grib2")).unwrap();
        let mut t = s.clone();
        t.values[0] = f32::NAN;
        t.values[1] = 9.0;
        let h = SeeingHour::from_fields(&s, &t, 0).unwrap();
        assert_eq!(h.cells[0], MISSING);
        assert_eq!(h.cells[1], MISSING);
        let mut other = t.clone();
        other.grid.ni += 1;
        assert!(SeeingHour::from_fields(&s, &other, 0).is_none());
    }

    #[test]
    fn factor_keeps_good_nights_and_cuts_haze() {
        assert_eq!(score_factor(5, 5), 1.0);
        assert_eq!(score_factor(0, 0), 1.0);
        assert!((score_factor(1, 1) - 0.425).abs() < 1e-9);
        assert!(score_factor(5, 2) < score_factor(2, 5));
    }

    #[test]
    fn file_names_follow_the_valid_hour() {
        let t = Utc.with_ymd_and_hms(2026, 11, 18, 6, 0, 0).unwrap();
        assert_eq!(file_name(t), "20261118T06Z.bin");
    }
}
