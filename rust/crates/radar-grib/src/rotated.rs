//! Rotated lat/lon grids (GRIB2 template 3.1), as ECCC's RDPS uses.
//!
//! A rotated grid is regular in a frame whose pole is moved. [`RotatedField`] keeps the values in
//! that frame and samples them at geographic points with [`RotatedField::sample`]: the point is
//! rotated into the grid frame, then read with bilinear interpolation. JPEG 2000 packing (5.40)
//! is unpacked by the `grib` crate's pure-Rust `hayro` backend.

use grib::def::grib2::GridDefinitionTemplate;
use grib::{Grib2SubmessageDecoder, SubMessage};

use crate::error::{GribError, Result};

/// The geometry of a rotated lat/lon grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotatedGrid {
    /// Points along a rotated parallel.
    pub ni: usize,
    /// Points along a rotated meridian.
    pub nj: usize,
    /// Rotated latitude of row 0, degrees.
    pub lat0: f64,
    /// Rotated longitude of column 0, degrees in [-180, 180).
    pub lon0: f64,
    /// Row step, degrees (negative when rows run north to south).
    pub dlat: f64,
    /// Column step, degrees.
    pub dlon: f64,
    /// The rotation.
    pub pole: RotatedPole,
}

/// One decoded message on a rotated lat/lon grid.
#[derive(Debug, Clone, PartialEq)]
pub struct RotatedField {
    /// Where the values sit.
    pub grid: RotatedGrid,
    /// Row-major `nj * ni`, row 0 first, NaN = missing.
    pub values: Vec<f32>,
}

/// The pole of a rotated grid, from the template's southern pole and rotation angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotatedPole {
    /// Latitude of the southern pole of the rotated frame, degrees.
    pub south_pole_lat: f64,
    /// Longitude of the southern pole of the rotated frame, degrees.
    pub south_pole_lon: f64,
    /// Rotation about the new polar axis, degrees.
    pub angle: f64,
}

impl RotatedPole {
    /// Geographic `(lat, lon)` in degrees to rotated `(lat, lon)`, longitude in [-180, 180].
    #[must_use]
    pub fn to_rotated(&self, lat: f64, lon: f64) -> (f64, f64) {
        // North pole of the rotated frame.
        let (sp, cp) = (-self.south_pole_lat).to_radians().sin_cos();
        let lon_p = self.south_pole_lon + 180.0;
        let (sf, cf) = lat.to_radians().sin_cos();
        let (sl, cl) = (lon - lon_p).to_radians().sin_cos();
        let (x, y, z) = (cf * cl, cf * sl, sf);
        // Tilt the pole onto the z axis.
        let xr = x * sp - z * cp;
        let zr = x * cp + z * sp;
        let lat_r = zr.clamp(-1.0, 1.0).asin().to_degrees();
        let lon_r = wrap180(y.atan2(xr).to_degrees() - 180.0 + self.angle);
        (lat_r, lon_r)
    }

    /// Rotated `(lat, lon)` in degrees to geographic `(lat, lon)`, longitude in [-180, 180].
    #[must_use]
    pub fn to_geographic(&self, lat_r: f64, lon_r: f64) -> (f64, f64) {
        let (sp, cp) = (-self.south_pole_lat).to_radians().sin_cos();
        let lon_p = self.south_pole_lon + 180.0;
        let (sf, cf) = lat_r.to_radians().sin_cos();
        let (sl, cl) = (lon_r - self.angle + 180.0).to_radians().sin_cos();
        let (xr, y, zr) = (cf * cl, cf * sl, sf);
        let x = xr * sp + zr * cp;
        let z = -xr * cp + zr * sp;
        let lat = z.clamp(-1.0, 1.0).asin().to_degrees();
        let lon = wrap180(y.atan2(x).to_degrees() + lon_p);
        (lat, lon)
    }
}

/// Wraps a longitude into [-180, 180).
#[inline]
fn wrap180(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

impl RotatedGrid {
    /// Geographic `(lat, lon)` of grid point `(row, col)`.
    #[must_use]
    pub fn latlon(&self, row: usize, col: usize) -> (f64, f64) {
        self.pole.to_geographic(
            self.lat0 + row as f64 * self.dlat,
            self.lon0 + col as f64 * self.dlon,
        )
    }

    /// Fractional `(row, col)` of a geographic point. May lie outside the grid.
    #[must_use]
    pub fn index_of(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (lat_r, lon_r) = self.pole.to_rotated(lat, lon);
        let col = wrap180(lon_r - self.lon0).rem_euclid(360.0) / self.dlon;
        ((lat_r - self.lat0) / self.dlat, col)
    }

    /// Nearest grid point `(row, col)` to a geographic point, or `None` outside the grid.
    #[must_use]
    pub fn nearest(&self, lat: f64, lon: f64) -> Option<(usize, usize)> {
        let (r, c) = self.index_of(lat, lon);
        let (r, c) = (r.round(), c.round());
        (r >= 0.0 && c >= 0.0 && (r as usize) < self.nj && (c as usize) < self.ni)
            .then_some((r as usize, c as usize))
    }
}

impl RotatedField {
    /// Value at grid point `(row, col)`.
    #[inline]
    #[must_use]
    pub fn at(&self, row: usize, col: usize) -> f32 {
        self.values[row * self.grid.ni + col]
    }

    /// Bilinear value at a geographic point. NaN outside the grid or when a corner is missing.
    #[must_use]
    pub fn sample(&self, lat: f64, lon: f64) -> f32 {
        let g = &self.grid;
        let (r, c) = g.index_of(lat, lon);
        let max_r = (g.nj - 1) as f64;
        let max_c = (g.ni - 1) as f64;
        if !(0.0..=max_r).contains(&r) || !(0.0..=max_c).contains(&c) {
            return f32::NAN;
        }
        let (r0, c0) = (
            (r.floor() as usize).min(g.nj.saturating_sub(2)),
            (c.floor() as usize).min(g.ni.saturating_sub(2)),
        );
        let (r1, c1) = ((r0 + 1).min(g.nj - 1), (c0 + 1).min(g.ni - 1));
        let (fr, fc) = (r - r0 as f64, c - c0 as f64);
        let v = |rr: usize, cc: usize| f64::from(self.at(rr, cc));
        let top = v(r0, c0) * (1.0 - fc) + v(r0, c1) * fc;
        let bottom = v(r1, c0) * (1.0 - fc) + v(r1, c1) * fc;
        (top * (1.0 - fr) + bottom * fr) as f32
    }
}

/// Decodes the first message of `bytes`, which must use template 3.1.
pub fn decode_rotated(bytes: &[u8]) -> Result<RotatedField> {
    let file = grib::from_bytes(bytes).map_err(|e| GribError::Parse(e.to_string()))?;
    let (_, sub) = file
        .iter()
        .next()
        .ok_or_else(|| GribError::Parse("file contains no GRIB2 messages".into()))?;
    rotated_submessage(sub)
}

fn rotated_submessage<R: grib::Grib2Read>(sub: SubMessage<'_, R>) -> Result<RotatedField> {
    let sec3 = sub
        .section3()
        .map_err(|e| GribError::Parse(format!("section 3: {e}")))?;
    let GridDefinitionTemplate::_3_1(t) = &sec3.payload.template else {
        return Err(GribError::Unsupported(format!(
            "expected rotated lat/lon (3.1), found 3.{}",
            sec3.payload.template_num
        )));
    };
    let g = &t.lat_lon.grid;
    let scan = t.lat_lon.scanning_mode.0;
    // Only i positive, rows consecutive in i, no alternating rows. j may run either way.
    if scan & 0x80 != 0 || scan & 0x20 != 0 || scan & 0x10 != 0 {
        return Err(GribError::Unsupported(format!(
            "rotated_ll scanning mode {scan:#04x}"
        )));
    }
    let unit = |v: f64| {
        crate::decode::grid_deg(
            v,
            g.initial_production_domain_basic_angle,
            g.basic_angle_subdivisions,
        )
    };
    let (ni, nj) = (g.ni as usize, g.nj as usize);
    if ni < 2 || nj < 2 {
        return Err(GribError::Unsupported(format!("rotated grid {ni} x {nj}")));
    }
    // Steps come from the corners: Di and Dj are rounded to micro-degrees.
    let lat0 = unit(f64::from(g.first_point_lat));
    let lat1 = unit(f64::from(g.last_point_lat));
    let lon0 = wrap180(unit(f64::from(g.first_point_lon)));
    let lon1 = wrap180(unit(f64::from(g.last_point_lon)));
    let grid = RotatedGrid {
        ni,
        nj,
        lat0,
        lon0,
        dlat: (lat1 - lat0) / (nj - 1) as f64,
        dlon: (lon1 - lon0).rem_euclid(360.0) / (ni - 1) as f64,
        pole: RotatedPole {
            south_pole_lat: unit(f64::from(t.rotation.south_pole_lat)),
            south_pole_lon: unit(f64::from(t.rotation.south_pole_lon)),
            angle: f64::from(t.rotation.rot_angle),
        },
    };
    let decoder = Grib2SubmessageDecoder::from(sub)
        .map_err(|e| GribError::Decode(format!("decoder setup: {e}")))?;
    let values: Vec<f32> = decoder
        .dispatch()
        .map_err(|e| GribError::Decode(format!("data representation: {e}")))?
        .collect();
    if values.len() != ni * nj {
        return Err(GribError::Decode(format!(
            "decoded {} values, grid has {} points",
            values.len(),
            ni * nj
        )));
    }
    Ok(RotatedField { grid, values })
}

#[cfg(test)]
mod tests {
    use super::*;
    use grib::LatLons;

    fn fixture(name: &str) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(name);
        std::fs::read(p).unwrap()
    }

    #[test]
    fn rotation_round_trips() {
        let pole = RotatedPole {
            south_pole_lat: -31.758_312,
            south_pole_lon: 267.597_031,
            angle: 0.0,
        };
        for (lat, lon) in [
            (42.33, -83.05),
            (49.28, -123.12),
            (25.76, -80.19),
            (64.0, -150.0),
        ] {
            let (r_lat, r_lon) = pole.to_rotated(lat, lon);
            let (lat2, lon2) = pole.to_geographic(r_lat, r_lon);
            assert!(
                (lat - lat2).abs() < 1e-9 && (lon - lon2).abs() < 1e-9,
                "{lat},{lon}"
            );
        }
    }

    #[test]
    fn seeing_fixture_decodes_with_cities_at_their_grid_points() {
        // Real RDPS 00Z 2026-11-18 PT006H, cut to rows 360-400, cols 665-715 by testdata/cut_eccc.py.
        let bytes = fixture("eccc_rdps_seeing_f006_greatlakes.grib2");
        let f = decode_rotated(&bytes).unwrap();
        assert_eq!((f.grid.ni, f.grid.nj), (51, 41));
        assert!(
            f.values
                .iter()
                .all(|v| (0.0..=5.0).contains(v) && v.fract() == 0.0)
        );
        // Our rotation matches the grib crate's own unrotated coordinates.
        let file = grib::from_bytes(&bytes).unwrap();
        let (_, sub) = file.iter().next().unwrap();
        let theirs: Vec<(f64, f64)> = sub.latlons_unchecked().unwrap().collect();
        for (r, c) in [(0, 0), (0, 50), (40, 0), (40, 50), (20, 25)] {
            let (lat, lon) = f.grid.latlon(r, c);
            let (tlat, tlon) = theirs[r * f.grid.ni + c];
            assert!((lat - tlat).abs() < 1e-6, "{r},{c}");
            assert!(wrap180(lon - tlon).abs() < 1e-6, "{r},{c}");
        }
        // Full-grid points: Detroit (370, 676) and Toronto (389, 703). GeoMet's
        // RDPS_10km_SeeingIndex GetFeatureInfo gave 3 at both for this run and hour.
        assert_eq!(f.grid.nearest(42.33, -83.05), Some((10, 11)));
        assert_eq!(f.grid.nearest(43.65, -79.38), Some((29, 38)));
        let (lat, lon) = f.grid.latlon(29, 38);
        assert!((lat - 43.6345).abs() < 1e-3 && (lon + 79.4375).abs() < 1e-3);
        assert_eq!(f.at(10, 11), 3.0);
        assert_eq!(f.at(29, 38), 3.0);
        assert_eq!(f.sample(43.65, -79.38), 3.0);
        assert!(f.sample(25.76, -80.19).is_nan(), "Miami is outside the cut");
    }

    #[test]
    fn transparency_fixture_decodes() {
        let f = decode_rotated(&fixture("eccc_rdps_transparency_f006_greatlakes.grib2")).unwrap();
        assert_eq!((f.grid.ni, f.grid.nj), (51, 41));
        assert_eq!(f.at(10, 11), 3.0);
        assert_eq!(f.at(29, 38), 3.0);
        assert!(f.values.iter().all(|v| (0.0..=5.0).contains(v)));
    }

    #[test]
    fn other_templates_are_refused() {
        let bytes = fixture("nbm_asnow_f006_utah.grib2");
        assert!(matches!(
            decode_rotated(&bytes),
            Err(GribError::Unsupported(_))
        ));
    }

    #[test]
    fn cut_matches_full_file() {
        // Manual check against the full file: ECCC_FULL=/path/to/SeeingIndex_EAtm.grib2.
        let Ok(path) = std::env::var("ECCC_FULL") else {
            return;
        };
        let full = decode_rotated(&std::fs::read(path).unwrap()).unwrap();
        let cut = decode_rotated(&fixture("eccc_rdps_seeing_f006_greatlakes.grib2")).unwrap();
        for r in 0..cut.grid.nj {
            for c in 0..cut.grid.ni {
                assert_eq!(cut.at(r, c), full.at(r + 360, c + 665), "{r},{c}");
                let (a, b) = (cut.grid.latlon(r, c), full.grid.latlon(r + 360, c + 665));
                assert!((a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4);
            }
        }
    }
}
