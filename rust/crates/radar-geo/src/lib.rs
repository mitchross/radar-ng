//! radar-geo: grid + projection types shared by the GRIB decoder (`radar-grib`), the tiler (`radar-tiler`),
//! nowcast and the API. OWNED BY THE INTEGRATOR — implementation agents must not change these
//! signatures (add new items in other modules instead).
//!
//! Conventions (mirrors the Python tiler contract, spec-tiler §1):
//! - `values` is row-major `height x width` f32, NaN = missing.
//! - Regular grids: `lats` (len = height) and `lons` (len = width) are 1-D axes in degrees.
//! - Projected grids: `x` (len = width) and `y` (len = height) are 1-D axes in source-CRS metres,
//!   plus full `lat_mesh`/`lon_mesh` (len = height*width) in EPSG:4326 degrees.

use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_4, PI};

/// Coordinate reference system of a projected source grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Crs {
    /// Spherical Lambert conformal conic (HRRR: R=6371229, lat_1=lat_2=lat_0=38.5, lon_0=-97.5).
    LambertConformal {
        lat_1: f64,
        lat_2: f64,
        lat_0: f64,
        lon_0: f64,
        radius_m: f64,
    },
    /// Spherical web mercator (EPSG:3857), R = 6378137. Used by tests.
    WebMercator,
}

impl Crs {
    /// Stable textual identity of the CRS (used in grid-spec digests and cache keys).
    pub fn identity_string(&self) -> String {
        match self {
            Crs::LambertConformal {
                lat_1,
                lat_2,
                lat_0,
                lon_0,
                radius_m,
            } => format!(
                "+proj=lcc +lat_1={lat_1:?} +lat_2={lat_2:?} +lat_0={lat_0:?} +lon_0={lon_0:?} +R={radius_m:?} +units=m +no_defs"
            ),
            Crs::WebMercator => "EPSG:3857".to_string(),
        }
    }

    /// Forward projection: (lon, lat) degrees -> (x, y) metres. Non-finite in -> non-finite out.
    pub fn forward(&self, lon: f64, lat: f64) -> (f64, f64) {
        match *self {
            Crs::WebMercator => {
                const R: f64 = 6_378_137.0;
                let x = R * lon.to_radians();
                let y = R * (FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln();
                (x, y)
            }
            Crs::LambertConformal {
                lat_1,
                lat_2,
                lat_0,
                lon_0,
                radius_m,
            } => {
                let p = LccParams::new(lat_1, lat_2, lat_0, lon_0, radius_m);
                let phi = lat.to_radians();
                let rho = radius_m * p.f / (FRAC_PI_4 + phi / 2.0).tan().powf(p.n);
                let mut dl = (lon - lon_0).to_radians();
                // normalise to [-pi, pi]
                while dl > PI {
                    dl -= 2.0 * PI;
                }
                while dl < -PI {
                    dl += 2.0 * PI;
                }
                let theta = p.n * dl;
                (rho * theta.sin(), p.rho0 - rho * theta.cos())
            }
        }
    }

    /// Inverse projection: (x, y) metres -> (lon, lat) degrees.
    pub fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        match *self {
            Crs::WebMercator => {
                const R: f64 = 6_378_137.0;
                let lon = (x / R).to_degrees();
                let lat = (2.0 * (y / R).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
                (lon, lat)
            }
            Crs::LambertConformal {
                lat_1,
                lat_2,
                lat_0,
                lon_0,
                radius_m,
            } => {
                let p = LccParams::new(lat_1, lat_2, lat_0, lon_0, radius_m);
                let dy = p.rho0 - y;
                let mut rho = (x * x + dy * dy).sqrt();
                let (xx, yy) = if p.n < 0.0 {
                    rho = -rho;
                    (-x, -dy)
                } else {
                    (x, dy)
                };
                let theta = xx.atan2(yy);
                let lat = if rho == 0.0 {
                    90.0_f64.copysign(p.n)
                } else {
                    (2.0 * (radius_m * p.f / rho).powf(1.0 / p.n).atan()
                        - std::f64::consts::FRAC_PI_2)
                        .to_degrees()
                };
                (lon_0 + (theta / p.n).to_degrees(), lat)
            }
        }
    }
}

struct LccParams {
    n: f64,
    f: f64,
    rho0: f64,
}

impl LccParams {
    fn new(lat_1: f64, lat_2: f64, lat_0: f64, _lon_0: f64, radius_m: f64) -> Self {
        let (p1, p2, p0) = (lat_1.to_radians(), lat_2.to_radians(), lat_0.to_radians());
        let t = |phi: f64| (FRAC_PI_4 + phi / 2.0).tan();
        let n = if (lat_1 - lat_2).abs() < 1e-10 {
            p1.sin()
        } else {
            (p1.cos() / p2.cos()).ln() / (t(p2) / t(p1)).ln()
        };
        let f = p1.cos() * t(p1).powf(n) / n;
        let rho0 = radius_m * f / t(p0).powf(n);
        Self { n, f, rho0 }
    }
}

/// Geolocation of a 2-D grid.
#[derive(Debug, Clone, PartialEq)]
pub enum GridGeo {
    /// Regular lat/lon grid with 1-D axes (MRMS, NAQFC-on-latlon, Open-Meteo-like).
    Regular { lats: Vec<f64>, lons: Vec<f64> },
    /// Projected grid (HRRR, NAQFC grid 227): 1-D source axes + full EPSG:4326 meshes.
    Projected {
        crs: Crs,
        x: Vec<f64>,
        y: Vec<f64>,
        lat_mesh: Vec<f64>,
        lon_mesh: Vec<f64>,
    },
}

/// A decoded 2-D field.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    /// Row-major `height * width`, NaN = missing.
    pub values: Vec<f32>,
    pub geo: GridGeo,
}

impl Grid {
    #[inline]
    pub fn at(&self, row: usize, col: usize) -> f32 {
        self.values[row * self.width + col]
    }

    /// Flip rows (north<->south) in place, including axes/meshes, mirroring the Python
    /// "flip when lats[0] > lats[-1]" / "source_y[0] > source_y[-1]" normalisation.
    pub fn flip_rows(&mut self) {
        let w = self.width;
        for r in 0..self.height / 2 {
            let (a, b) = (r * w, (self.height - 1 - r) * w);
            for c in 0..w {
                self.values.swap(a + c, b + c);
            }
        }
        match &mut self.geo {
            GridGeo::Regular { lats, .. } => lats.reverse(),
            GridGeo::Projected {
                y,
                lat_mesh,
                lon_mesh,
                ..
            } => {
                y.reverse();
                for m in [lat_mesh, lon_mesh] {
                    for r in 0..self.height / 2 {
                        let (a, b) = (r * w, (self.height - 1 - r) * w);
                        for c in 0..w {
                            m.swap(a + c, b + c);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lcc_roundtrip() {
        let c = Crs::LambertConformal {
            lat_1: 38.5,
            lat_2: 38.5,
            lat_0: 38.5,
            lon_0: -97.5,
            radius_m: 6371229.0,
        };
        for (lon, lat) in [
            (-122.7, 21.1),
            (-97.5, 38.5),
            (-60.9, 47.8),
            (-85.67, 42.96),
        ] {
            let (x, y) = c.forward(lon, lat);
            let (lo, la) = c.inverse(x, y);
            assert!(
                (lo - lon).abs() < 1e-9 && (la - lat).abs() < 1e-9,
                "{lon},{lat} -> {lo},{la}"
            );
        }
        let (x, y) = c.forward(-97.5, 38.5);
        assert!(x.abs() < 1e-6 && y.abs() < 1e-6);
    }
    #[test]
    fn mercator_roundtrip() {
        let (x, y) = Crs::WebMercator.forward(-85.0, 42.0);
        let (lo, la) = Crs::WebMercator.inverse(x, y);
        assert!((lo + 85.0).abs() < 1e-9 && (la - 42.0).abs() < 1e-9);
    }
}
