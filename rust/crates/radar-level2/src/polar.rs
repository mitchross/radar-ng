use crate::decode::{BLANKED, FOLDED, Sweep, UNOBSERVED};
use anyhow::{Result, bail, ensure};
use radar_tiler::{ClassModel, png::IndexedTileEncoder};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_GROUND_RANGE_M: f64 = 150_000.0;
pub const TILE: usize = 256;
pub const SAMPLER_VERSION: &str = "level2-polar-v1";
const EARTH_RADIUS_M: f64 = 6_371_000.0;
const K: f64 = EARTH_RADIUS_M * 4.0 / 3.0;
const WGS_A: f64 = 6_378_137.0;
const WGS_F: f64 = 1.0 / 298.257_223_563;
/// Tiles whose pixels exceed this ground size minify with 2x2 subpixels.
const MINIFY_PIXEL_M: f64 = 125.0;
const PNG_LEVEL: u32 = 6;
/// Bounds every buffer a tile render allocates; a test proves actual use stays below it.
pub const TILE_SCRATCH_BYTES: usize = 1024 * 1024;

/// Slant range for ground distance `ground_m` on the 4/3 effective Earth.
pub fn ground_to_slant(ground_m: f64, tilt_deg: f64) -> Option<f64> {
    let d = ground_m / K;
    let den = (tilt_deg.to_radians() + d).cos();
    (den > 1e-6).then(|| K * d.sin() / den)
}

/// Geodesic distance (m) and initial azimuth (deg) on WGS84, by Vincenty's inverse formula.
pub fn inverse_wgs84(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> Result<(f64, f64)> {
    let b = WGS_A * (1.0 - WGS_F);
    let u1 = ((1.0 - WGS_F) * lat1.to_radians().tan()).atan();
    let u2 = ((1.0 - WGS_F) * lat2.to_radians().tan()).atan();
    let l = (lon2 - lon1).to_radians();
    let l = l.sin().atan2(l.cos());
    let (su1, cu1, su2, cu2) = (u1.sin(), u1.cos(), u2.sin(), u2.cos());
    let mut lambda = l;
    for _ in 0..100 {
        let (sl, cl) = lambda.sin_cos();
        let ss = (cu2 * sl).hypot(cu1 * su2 - su1 * cu2 * cl);
        if ss == 0.0 {
            return Ok((0.0, 0.0));
        }
        let cs = su1 * su2 + cu1 * cu2 * cl;
        let sigma = ss.atan2(cs);
        let sa = cu1 * cu2 * sl / ss;
        let c2a = 1.0 - sa * sa;
        let c2sm = if c2a == 0.0 {
            0.0
        } else {
            cs - 2.0 * su1 * su2 / c2a
        };
        let c = WGS_F / 16.0 * c2a * (4.0 + WGS_F * (4.0 - 3.0 * c2a));
        let next = l
            + (1.0 - c)
                * WGS_F
                * sa
                * (sigma + c * ss * (c2sm + c * cs * (-1.0 + 2.0 * c2sm * c2sm)));
        if (next - lambda).abs() < 1e-12 {
            let u_sq = c2a * (WGS_A * WGS_A - b * b) / (b * b);
            let a =
                1.0 + u_sq / 16384.0 * (4096.0 + u_sq * (-768.0 + u_sq * (320.0 - 175.0 * u_sq)));
            let bb = u_sq / 1024.0 * (256.0 + u_sq * (-128.0 + u_sq * (74.0 - 47.0 * u_sq)));
            let ds = bb
                * ss
                * (c2sm
                    + bb / 4.0
                        * (cs * (-1.0 + 2.0 * c2sm * c2sm)
                            - bb / 6.0
                                * c2sm
                                * (-3.0 + 4.0 * ss * ss)
                                * (-3.0 + 4.0 * c2sm * c2sm)));
            let (sl, cl) = next.sin_cos();
            let az = (cu2 * sl).atan2(cu1 * su2 - su1 * cu2 * cl).to_degrees();
            return Ok((b * a * (sigma - ds), az.rem_euclid(360.0)));
        }
        lambda = next;
    }
    bail!("geodesic inverse did not converge")
}

/// One resident REF sweep: values, QC and per-ray geometry. Memory is [`PolarSweep::bytes`].
pub struct PolarSweep {
    pub identity: String,
    site: (f64, f64),
    azimuth: Vec<f32>,
    elevation: Vec<f32>,
    spacing: Vec<f32>,
    first_gate: Vec<i16>,
    gate_size: Vec<i16>,
    offsets: Vec<u32>,
    values: Vec<f32>,
    qc: Vec<u8>,
    order: Vec<u16>,
}

pub struct RayInput {
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    pub spacing_deg: f32,
    pub first_gate_m: i16,
    pub gate_m: i16,
    pub gates: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gate {
    Missing,
    Clear,
    Dbz(f32),
}

pub struct Hit {
    pub ray: usize,
    pub gate: usize,
    pub slant_m: f64,
}

impl PolarSweep {
    /// Mean antenna tilt over all rays.
    pub fn mean_elevation_deg(&self) -> Option<f64> {
        let n = self.elevation.len();
        (n > 0).then(|| self.elevation.iter().map(|e| f64::from(*e)).sum::<f64>() / n as f64)
    }

    pub fn new(
        identity: String,
        site: (f64, f64),
        rays: &[RayInput],
        values: Vec<f32>,
        qc: Vec<u8>,
    ) -> Result<Self> {
        ensure!(
            !rays.is_empty() && rays.len() <= crate::RADIAL_LIMIT && values.len() == qc.len(),
            "polar sweep shape"
        );
        let mut offsets = Vec::with_capacity(rays.len() + 1);
        offsets.push(0u32);
        for r in rays {
            ensure!(
                r.gates <= crate::GATE_LIMIT
                    && r.azimuth_deg.is_finite()
                    && r.elevation_deg.is_finite()
                    && (r.gates == 0 || r.gate_m > 0),
                "polar ray bounds"
            );
            offsets.push(offsets[offsets.len() - 1] + r.gates as u32);
        }
        ensure!(
            offsets[rays.len()] as usize == values.len(),
            "polar gate count"
        );
        let mut order: Vec<u16> = (0..rays.len() as u16).collect();
        order.sort_by(|a, b| {
            rays[*a as usize]
                .azimuth_deg
                .rem_euclid(360.0)
                .total_cmp(&rays[*b as usize].azimuth_deg.rem_euclid(360.0))
        });
        Ok(Self {
            identity,
            site,
            azimuth: rays
                .iter()
                .map(|r| r.azimuth_deg.rem_euclid(360.0))
                .collect(),
            elevation: rays.iter().map(|r| r.elevation_deg).collect(),
            spacing: rays.iter().map(|r| r.spacing_deg).collect(),
            first_gate: rays.iter().map(|r| r.first_gate_m).collect(),
            gate_size: rays.iter().map(|r| r.gate_m).collect(),
            offsets,
            values,
            qc,
            order,
        })
    }

    pub fn from_sweep(sweep: &Sweep, identity: &str) -> Result<Self> {
        let site = sweep.site.ok_or_else(|| anyhow::anyhow!("missing site"))?;
        let (mut rays, mut values, mut qc) = (vec![], vec![], vec![]);
        for r in &sweep.rays {
            let m = r.moments[0].as_ref();
            rays.push(RayInput {
                azimuth_deg: r.azimuth_deg,
                elevation_deg: r.elevation_deg,
                spacing_deg: r.azimuth_spacing_deg,
                first_gate_m: m.map_or(0, |m| m.first_gate_m),
                gate_m: m.map_or(0, |m| m.spacing_m),
                gates: m.map_or(0, |m| m.values.len()),
            });
            if let Some(m) = m {
                values.extend_from_slice(&m.values);
                qc.extend_from_slice(&m.qc);
            }
        }
        Self::new(
            identity.into(),
            (f64::from(site.latitude_deg), f64::from(site.longitude_deg)),
            &rays,
            values,
            qc,
        )
    }

    /// Heap bytes held: gates, QC, per-ray geometry and the azimuth index.
    pub fn bytes(&self) -> usize {
        self.values.capacity() * 4
            + self.qc.capacity()
            + self.azimuth.capacity() * 4
            + self.elevation.capacity() * 4
            + self.spacing.capacity() * 4
            + self.first_gate.capacity() * 2
            + self.gate_size.capacity() * 2
            + self.offsets.capacity() * 4
            + self.order.capacity() * 2
            + self.identity.capacity()
            + std::mem::size_of::<Self>()
    }

    fn angle(a: f64, b: f64) -> f64 {
        (b - a).rem_euclid(360.0)
    }

    /// The ray that owns azimuth `az`. Rays across a gap never stretch past half their spacing.
    fn owner(&self, az: f64) -> Option<usize> {
        let n = self.order.len();
        let at = |i: usize| f64::from(self.azimuth[self.order[i] as usize]);
        let next = self
            .order
            .partition_point(|r| f64::from(self.azimuth[*r as usize]) <= az)
            % n;
        let prev = (next + n - 1) % n;
        let (p, q) = (self.order[prev] as usize, self.order[next] as usize);
        let back = Self::angle(at(prev), az);
        let fwd = Self::angle(az, at(next));
        let (ray, dist) = if back <= fwd { (p, back) } else { (q, fwd) };
        let span = f64::from(self.spacing[p].max(self.spacing[q]));
        let gap = if n == 1 { 360.0 } else { back + fwd };
        let half = f64::from(self.spacing[ray]) / 2.0;
        (gap <= 1.5 * span || dist <= half + 1e-4).then_some(ray)
    }

    fn gate_at(&self, ground_m: f64, az: f64) -> Option<(Gate, Hit)> {
        let ray = self.owner(az)?;
        let slant = ground_to_slant(ground_m, f64::from(self.elevation[ray]))?;
        let size = f64::from(self.gate_size[ray]);
        let (lo, hi) = (self.offsets[ray] as usize, self.offsets[ray + 1] as usize);
        if hi == lo {
            return None;
        }
        let index = ((slant - f64::from(self.first_gate[ray])) / size + 0.5).floor();
        if index < 0.0 || index >= (hi - lo) as f64 {
            return None;
        }
        let gate = lo + index as usize;
        let (v, q) = (self.values[gate], self.qc[gate]);
        let state = if q == 0 && v.is_finite() {
            Gate::Dbz(v)
        } else if q & (FOLDED | BLANKED | UNOBSERVED) == 0 && q != 0 {
            Gate::Clear
        } else {
            Gate::Missing
        };
        Some((
            state,
            Hit {
                ray,
                gate,
                slant_m: slant,
            },
        ))
    }

    /// The gate under a geographic point, or `None` outside the 150 km mask, a gap or the gates.
    pub fn locate(&self, lat: f64, lon: f64) -> Option<Hit> {
        let (ground, az) = inverse_wgs84(self.site.0, self.site.1, lat, lon).ok()?;
        (ground <= MAX_GROUND_RANGE_M).then_some(())?;
        self.gate_at(ground, az).map(|(_, hit)| hit)
    }

    pub fn sample(&self, lat: f64, lon: f64) -> Gate {
        let Ok((ground, az)) = inverse_wgs84(self.site.0, self.site.1, lat, lon) else {
            return Gate::Missing;
        };
        if ground > MAX_GROUND_RANGE_M {
            return Gate::Missing;
        }
        self.gate_at(ground, az).map_or(Gate::Missing, |(g, _)| g)
    }

    fn coarse_km(&self, lat: f64, lon: f64) -> f64 {
        let dy = (lat - self.site.0) * 111.32;
        let dx = (lon - self.site.1).rem_euclid(360.0);
        let dx = if dx > 180.0 { dx - 360.0 } else { dx } * 111.32 * self.site.0.to_radians().cos();
        dx.hypot(dy)
    }
}

/// Global Web Mercator pixel to longitude/latitude.
fn pixel_lon_lat(px: f64, py: f64, world: f64) -> (f64, f64) {
    let lon = px / world * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * py / world))
        .sinh()
        .atan()
        .to_degrees();
    (lat, lon)
}

pub fn pixel_metres(z: u32, lat: f64) -> f64 {
    2.0 * std::f64::consts::PI * WGS_A * lat.to_radians().cos()
        / (TILE as f64 * f64::from(1u32 << z))
}

/// Reusable per-render buffers; [`Scratch::bytes`] reports their real capacity.
pub struct Scratch {
    pub dbz: Vec<f32>,
    classes: Vec<u8>,
    encoder: IndexedTileEncoder,
    tail: Vec<u8>,
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            dbz: Vec::with_capacity(TILE * TILE),
            classes: Vec::with_capacity(TILE * TILE),
            encoder: IndexedTileEncoder::new(PNG_LEVEL),
            tail: Vec::new(),
        }
    }
}

impl Scratch {
    pub fn bytes(&self) -> usize {
        self.dbz.capacity() * 4
            + self.classes.capacity()
            + self.tail.capacity()
            + 4 * TILE * (TILE + 1)
    }
}

pub struct Rendered {
    pub png: Vec<u8>,
    pub transparent: bool,
}

/// Mean linear Z over supported samples as dBZ; `None` when nothing but missing or clear air.
fn combine(samples: &[Gate]) -> Option<f32> {
    let mut values = samples
        .iter()
        .filter_map(|s| if let Gate::Dbz(v) = s { Some(*v) } else { None });
    if let Some(first) = values.next()
        && samples
            .iter()
            .all(|s| matches!(s, Gate::Missing) || *s == Gate::Dbz(first))
    {
        return Some(first);
    }
    let (mut sum, mut n) = (0.0f64, 0u32);
    for s in samples {
        match s {
            Gate::Dbz(v) => {
                sum += 10f64.powf(f64::from(*v) / 10.0);
                n += 1;
            }
            Gate::Clear => n += 1,
            Gate::Missing => {}
        }
    }
    (n > 0 && sum > 0.0).then(|| (10.0 * (sum / f64::from(n)).log10()) as f32)
}

impl PolarSweep {
    /// Fill `out` (256x256 dBZ, NaN where transparent) for tile `z/x/y`; `false` if cancelled.
    pub fn sample_tile(
        &self,
        z: u32,
        x: u32,
        y: u32,
        cancel: &AtomicBool,
        out: &mut Vec<f32>,
    ) -> bool {
        out.clear();
        out.resize(TILE * TILE, f32::NAN);
        let world = (TILE as f64) * f64::from(1u32 << z);
        let (mid_lat, _) = pixel_lon_lat(0.0, (f64::from(y) + 0.5) * TILE as f64, world);
        let offsets: &[f64] = if pixel_metres(z, mid_lat) > MINIFY_PIXEL_M {
            &[0.25, 0.75]
        } else {
            &[0.5]
        };
        let near =
            |lat: f64, lon: f64| self.coarse_km(lat, lon) <= MAX_GROUND_RANGE_M / 1000.0 + 5.0;
        let n_lat = pixel_lon_lat(0.0, f64::from(y) * TILE as f64, world).0;
        let s_lat = pixel_lon_lat(0.0, (f64::from(y) + 1.0) * TILE as f64, world).0;
        let (w_lon, e_lon) = (
            pixel_lon_lat(f64::from(x) * TILE as f64, 0.0, world).1,
            pixel_lon_lat((f64::from(x) + 1.0) * TILE as f64, 0.0, world).1,
        );
        let (clat, clon) = (
            self.site.0.clamp(s_lat, n_lat),
            self.site.1.clamp(w_lon, e_lon),
        );
        if !near(clat, clon) {
            return true;
        }
        let mut samples = [Gate::Missing; 4];
        for row in 0..TILE {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            for col in 0..TILE {
                let mut n = 0;
                for dy in offsets {
                    for dx in offsets {
                        let px = f64::from(x) * TILE as f64 + col as f64 + dx;
                        let py = f64::from(y) * TILE as f64 + row as f64 + dy;
                        let (lat, lon) = pixel_lon_lat(px, py, world);
                        samples[n] = if near(lat, lon) {
                            self.sample(lat, lon)
                        } else {
                            Gate::Missing
                        };
                        n += 1;
                    }
                }
                if let Some(v) = combine(&samples[..n]) {
                    out[row * TILE + col] = v;
                }
            }
        }
        true
    }

    // Tile coordinates, model, palette, cancel and scratch are all required inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn render_tile(
        &self,
        z: u32,
        x: u32,
        y: u32,
        model: &ClassModel,
        palette: &str,
        cancel: &AtomicBool,
        scratch: &mut Scratch,
    ) -> Result<Rendered> {
        ensure!(
            (8..=13).contains(&z) && x < 1 << z && y < 1 << z,
            "invalid tile"
        );
        ensure!(
            self.sample_tile(z, x, y, cancel, &mut scratch.dbz),
            "render cancelled"
        );
        encode_tile(model, palette, scratch)
    }
}

/// Classify the sampled dBZ in `scratch` with the shared MRMS reflectivity model and encode the palette PNG.
pub fn encode_tile(model: &ClassModel, palette: &str, scratch: &mut Scratch) -> Result<Rendered> {
    let lut = model
        .lut(palette)
        .ok_or_else(|| anyhow::anyhow!("unknown palette"))?;
    scratch.classes.clear();
    scratch.classes.resize(TILE * TILE, 0);
    model.classify_f32_into(&scratch.dbz, &mut scratch.classes)?;
    let transparent = scratch.classes.iter().all(|c| *c == 0);
    let depth = radar_tiler::png::bit_depth_for_entries(lut.rgba().len());
    scratch
        .encoder
        .encode_tail(&scratch.classes, TILE, TILE, depth, &mut scratch.tail)?;
    let mut png =
        Vec::with_capacity(scratch.tail.len() + lut.plte().len() + lut.trns().len() + 128);
    png.extend_from_slice(&radar_tiler::png::header_bytes(
        TILE as u32,
        TILE as u32,
        depth,
    ));
    radar_tiler::png::append_palette_chunks(&mut png, lut.plte(), lut.trns());
    png.extend_from_slice(&scratch.tail);
    Ok(Rendered { png, transparent })
}

/// One shared class model over every shipped palette, exactly as MRMS builds reflectivity.
pub fn reflectivity_model() -> Result<ClassModel> {
    let mut tables = vec![];
    for name in ["classic", "muted", "vivid"] {
        tables.push((
            name.to_owned(),
            radar_tiler::load_palette(name)?.color_table("reflectivity")?,
        ));
    }
    Ok(radar_tiler::build_class_model(&tables)?)
}

pub fn palette_version(model: &ClassModel) -> String {
    let mut text = format!("{:?}{:?}", model.edges(), model.remap());
    for name in ["classic", "muted", "vivid"] {
        if let Some(l) = model.lut(name) {
            text += &format!("{name}{:?}", l.rgba());
        }
    }
    crate::checksum(text.as_bytes())[..16].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_z_keeps_cores_and_never_counts_missing_as_clear() {
        let core = combine(&[Gate::Dbz(60.0), Gate::Clear, Gate::Clear, Gate::Clear]).unwrap();
        assert!((core - 53.98).abs() < 0.01, "{core}");
        assert_eq!(
            combine(&[Gate::Dbz(40.0), Gate::Missing, Gate::Missing, Gate::Missing]),
            Some(40.0)
        );
        assert_eq!(
            combine(&[
                Gate::Dbz(-5.0),
                Gate::Dbz(-5.0),
                Gate::Missing,
                Gate::Dbz(-5.0)
            ]),
            Some(-5.0)
        );
        assert_eq!(combine(&[Gate::Missing; 4]), None);
        assert_eq!(
            combine(&[Gate::Clear, Gate::Missing, Gate::Clear, Gate::Missing]),
            None
        );
        let mixed = combine(&[Gate::Dbz(-10.0), Gate::Dbz(20.0)]).unwrap();
        assert!((mixed - 16.99).abs() < 0.01, "{mixed}");
    }
}
