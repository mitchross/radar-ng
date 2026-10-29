//! HRRR grid dumps (spec §7.5): resample a projected field onto a regular 0.03 degree grid, and rotate
//! grid-relative winds to earth-relative (spec §5.5). Pure except for the final `write_grid` call.

use radar_core::grid_dump::{self, GridInput};
use radar_geo::{Crs, Grid, GridGeo};
use radar_tiler::geometry::AxisMap;

use crate::config::IngestConfig;

/// Projection step (metres) used to measure the grid-x direction for wind rotation (spec §5.5).
pub const THETA_STEP_M: f64 = 1000.0;

/// Nan-aware min and max of a slice (NaN values skipped). `None` when no finite value.
fn finite_min_max(values: &[f64]) -> Option<(f64, f64)> {
    let mut it = values.iter().copied().filter(|v| v.is_finite());
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
}

/// numpy `arange(start, stop, step)`: `ceil((stop - start) / step)` points, `start + i * step`.
pub fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil().max(0.0) as usize;
    (0..n).map(|i| start + i as f64 * step).collect()
}

/// Target axes `(lats, lons)` for a projected grid (spec §7.5): `arange(min, max, step)` of the 2-D
/// latitude and longitude meshes. The end is excluded, as numpy does.
pub fn target_axes(grid: &Grid, step: f64) -> anyhow::Result<(Vec<f64>, Vec<f64>)> {
    let GridGeo::Projected {
        lat_mesh, lon_mesh, ..
    } = &grid.geo
    else {
        anyhow::bail!("HRRR dumps need a projected grid");
    };
    let (lat_lo, lat_hi) =
        finite_min_max(lat_mesh).ok_or_else(|| anyhow::anyhow!("no finite latitude"))?;
    let (lon_lo, lon_hi) =
        finite_min_max(lon_mesh).ok_or_else(|| anyhow::anyhow!("no finite longitude"))?;
    Ok((arange(lat_lo, lat_hi, step), arange(lon_lo, lon_hi, step)))
}

/// Maps a regular target grid into source fractional indices, with grid-x rotation angles.
#[derive(Debug, Clone)]
pub struct Resampler {
    out_h: usize,
    out_w: usize,
    src_h: usize,
    src_w: usize,
    rows: Vec<f64>,
    cols: Vec<f64>,
    theta: Vec<f64>,
}

impl Resampler {
    /// Build the resampler for target axes `lats` x `lons` over a source with axes `source_x`/`source_y`.
    pub fn new(
        crs: &Crs,
        source_x: &[f64],
        source_y: &[f64],
        src_h: usize,
        src_w: usize,
        lats: &[f64],
        lons: &[f64],
    ) -> anyhow::Result<Self> {
        let map_x = AxisMap::new(source_x)?;
        let map_y = AxisMap::new(source_y)?;
        let (out_h, out_w) = (lats.len(), lons.len());
        let mut rows = Vec::with_capacity(out_h * out_w);
        let mut cols = Vec::with_capacity(out_h * out_w);
        let mut theta = Vec::with_capacity(out_h * out_w);
        for &lat in lats {
            for &lon in lons {
                let (x, y) = crs.forward(lon, lat);
                rows.push(map_y.map(y));
                cols.push(map_x.map(x));
                theta.push(grid_x_angle(crs, lon, lat));
            }
        }
        Ok(Self {
            out_h,
            out_w,
            src_h,
            src_w,
            rows,
            cols,
            theta,
        })
    }

    /// Output shape `(height, width)`.
    pub fn shape(&self) -> (usize, usize) {
        (self.out_h, self.out_w)
    }

    /// Bilinear sample of a source field (f32 in, f32 out). Out-of-range coordinates and non-finite
    /// neighbours with positive weight give NaN (`map_coordinates(order=1, mode="constant", cval=NaN)`).
    pub fn resample(&self, src: &[f32]) -> Vec<f32> {
        debug_assert_eq!(src.len(), self.src_h * self.src_w);
        let (h, w) = (self.src_h, self.src_w);
        (0..self.out_h * self.out_w)
            .map(|k| {
                let (r, c) = (self.rows[k], self.cols[k]);
                if !r.is_finite()
                    || !c.is_finite()
                    || r < 0.0
                    || c < 0.0
                    || r > (h - 1) as f64
                    || c > (w - 1) as f64
                {
                    return f32::NAN;
                }
                let r0 = r.floor() as usize;
                let c0 = c.floor() as usize;
                let r1 = (r0 + 1).min(h - 1);
                let c1 = (c0 + 1).min(w - 1);
                let fr = r - r0 as f64;
                let fc = c - c0 as f64;
                let corners = [
                    ((r0, c0), (1.0 - fr) * (1.0 - fc)),
                    ((r0, c1), (1.0 - fr) * fc),
                    ((r1, c0), fr * (1.0 - fc)),
                    ((r1, c1), fr * fc),
                ];
                let mut acc = 0.0f64;
                for ((rr, cc), weight) in corners {
                    if weight == 0.0 {
                        continue;
                    }
                    let v = src[rr * w + cc];
                    if !v.is_finite() {
                        return f32::NAN;
                    }
                    acc += weight * v as f64;
                }
                acc as f32
            })
            .collect()
    }

    /// Rotate resampled `(u, v)` by the grid-x angle (earth-relative), element-wise.
    pub fn rotate(&self, u: &[f32], v: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut ou = Vec::with_capacity(u.len());
        let mut ov = Vec::with_capacity(v.len());
        for k in 0..u.len() {
            let (s, c) = self.theta[k].sin_cos();
            let (uu, vv) = (u[k] as f64, v[k] as f64);
            ou.push((uu * c - vv * s) as f32);
            ov.push((uu * s + vv * c) as f32);
        }
        (ou, ov)
    }

    /// Resampled `(u, v)` for a wind pair. Rotated when `grid_relative`, otherwise resampled only.
    pub fn wind_components(
        &self,
        u: &[f32],
        v: &[f32],
        grid_relative: bool,
    ) -> (Vec<f32>, Vec<f32>) {
        let ru = self.resample(u);
        let rv = self.resample(v);
        if grid_relative {
            self.rotate(&ru, &rv)
        } else {
            (ru, rv)
        }
    }
}

/// Angle of the grid x-axis measured from east at `(lon, lat)`, in radians (spec §5.5).
pub fn grid_x_angle(crs: &Crs, lon: f64, lat: f64) -> f64 {
    let (x, y) = crs.forward(lon, lat);
    let (lon_e, lat_e) = crs.inverse(x + THETA_STEP_M, y);
    (lat_e - lat).atan2((lon_e - lon) * lat.to_radians().cos())
}

/// Write one resampled field as a regular grid dump, keyed by valid time (spec §7.5 `write_grid`).
pub fn write_dump(
    cfg: &IngestConfig,
    layer: &str,
    valid: &str,
    data: &[f32],
    lats: &[f64],
    lons: &[f64],
    unit: &str,
) -> anyhow::Result<()> {
    let input = GridInput::new(data, lats, lons, unit);
    grid_dump::write_grid(&cfg.grid, layer, valid, &input)?;
    Ok(())
}

/// `_safe_grid_dump`: logs `grid_dump_failed` (with `layer`) and never raises.
pub fn safe_dump(
    cfg: &IngestConfig,
    layer: &str,
    valid: &str,
    data: &[f32],
    lats: &[f64],
    lons: &[f64],
    unit: &str,
) {
    if let Err(err) = write_dump(cfg, layer, valid, data, lats, lons, unit) {
        tracing::warn!(err = %err, layer = %layer, "grid_dump_failed");
    }
}
