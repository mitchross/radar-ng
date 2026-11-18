//! Archive hazard-case QC review; run via `rust/scripts/level2-qc-report.sh`, see docs/develop/level2-qc-review.md.
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use radar_level2::{
    artifact::{self, Artifact},
    decode::{BELOW, BLANKED, Decoder, FOLDED, Moment, Sweep, capped_read},
    feed::Source,
    polar::{Gate, MAX_GROUND_RANGE_M, PolarSweep, TILE, reflectivity_model},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

/// Panel width in pixels and pixel size: ±150 km at 300 m covers the served range.
const PANEL_PX: usize = 1000;
const PANEL_M: f64 = 300.0;
/// Residual-clutter proxy: retained REF with RHO below this is non-meteorological-looking.
const LOW_RHO: f32 = 0.80;
/// A hole is a small non-echo region whose every neighbour is echo at least this strong.
const HOLE_EDGE_DBZ: f32 = 15.0;
const HOLE_MAX_GATES: usize = 200;

#[derive(Parser)]
struct Cli {
    /// Case list in the corpus.json shape (`testdata/hazard-cases.json`).
    #[arg(long)]
    cases: Option<PathBuf>,
    /// Directory holding the downloaded archive objects (and `.gz` cases unpacked beside them).
    #[arg(long)]
    archive: PathBuf,
    #[arg(long)]
    out: PathBuf,
    /// Ad hoc `STATION:FILE` volumes, no checksum; for screening candidate cases.
    #[arg(long)]
    file: Vec<String>,
    /// Only run cases whose id contains this text.
    #[arg(long)]
    only: Option<String>,
    #[arg(long)]
    no_panels: bool,
    /// Gate counts only (no artifact, grid or panels); for screening many volumes.
    #[arg(long)]
    screen: bool,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    category: Vec<String>,
    #[serde(default)]
    station: String,
    file: Option<String>,
    bytes: Option<usize>,
    sha256: Option<String>,
    url: Option<String>,
}

struct Raw {
    code: u16,
    dbz: f32,
    blanked: bool,
}

fn raw_gate(m: &Moment, gate: usize, blanking: u8) -> Raw {
    let code = if m.word_bits == 8 {
        u16::from(m.encoded[gate])
    } else {
        u16::from_be_bytes([m.encoded[2 * gate], m.encoded[2 * gate + 1]])
    };
    Raw {
        code,
        dbz: (f32::from(code) - m.offset) / m.scale,
        blanked: blanking != 0,
    }
}

fn valid(r: &Raw) -> bool {
    r.code >= 2 && !r.blanked
}

fn slant_m(m: &Moment, gate: usize) -> f64 {
    f64::from(m.first_gate_m) + gate as f64 * f64::from(m.spacing_m)
}

fn rho_at(sweep: &Sweep, ray: usize, slant: f64) -> Option<f32> {
    let m = sweep.rays[ray].moments[1].as_ref()?;
    let i = ((slant - f64::from(m.first_gate_m)) / f64::from(m.spacing_m)).round();
    (i >= 0.0 && (i as usize) < m.values.len())
        .then(|| m.values[i as usize])
        .filter(|v| v.is_finite())
}

/// Completed lowest sweeps, plus the `finish` error that makes the archive path reject them.
fn decode(station: &str, bytes: &[u8]) -> Result<(Vec<Sweep>, Option<String>)> {
    let mut d = Decoder::new(station);
    let sweeps = d.push(bytes)?;
    Ok((sweeps, d.finish().err().map(|e| format!("{e:#}"))))
}

/// The shipped serving path: artifact build, publish with read-back, verified load.
async fn served(sweep: &Sweep, source: &Source) -> Result<PolarSweep> {
    let store = object_store::memory::InMemory::new();
    let a = Artifact::build(sweep, vec![source.clone()], "level2/review")?;
    a.publish(&store).await?;
    Ok(artifact::load(&store, &a.commit_key).await?.1)
}

struct Grid {
    site: (f64, f64),
}

impl Grid {
    fn lat_lon(&self, row: usize, col: usize) -> (f64, f64) {
        let half = PANEL_PX as f64 / 2.0;
        let dy = (half - row as f64 - 0.5) * PANEL_M / 1000.0;
        let dx = (col as f64 + 0.5 - half) * PANEL_M / 1000.0;
        (
            self.site.0 + dy / 111.32,
            self.site.1 + dx / (111.32 * self.site.0.to_radians().cos()),
        )
    }
}

/// Approximate destination of a gate, good enough to start a local search for its footprint.
fn gate_lat_lon(site: (f64, f64), az_deg: f64, ground_m: f64) -> (f64, f64) {
    let d = ground_m / 6_371_000.0;
    let (lat1, lon1, az) = (
        site.0.to_radians(),
        site.1.to_radians(),
        az_deg.to_radians(),
    );
    let lat2 = (lat1.sin() * d.cos() + lat1.cos() * d.sin() * az.cos()).asin();
    let lon2 = lon1 + (az.sin() * d.sin() * lat1.cos()).atan2(d.cos() - lat1.sin() * lat2.sin());
    (lat2.to_degrees(), lon2.to_degrees())
}

fn tile_of(lat: f64, lon: f64, z: u32) -> (u32, u32) {
    let n = f64::from(1u32 << z);
    let x = ((lon + 180.0) / 360.0 * n).floor();
    let y = ((1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor();
    (x as u32, y as u32)
}

fn tile_values(s: &PolarSweep, lat: f64, lon: f64, z: u32) -> Vec<f32> {
    let (x, y) = tile_of(lat, lon, z);
    let mut out = Vec::with_capacity(TILE * TILE);
    s.sample_tile(z, x, y, &AtomicBool::new(false), &mut out);
    out
}

fn max_finite(v: impl IntoIterator<Item = f32>) -> Option<f32> {
    v.into_iter().filter(|v| v.is_finite()).reduce(f32::max)
}

#[derive(Default)]
struct Counts {
    gates: u64,
    valid: u64,
    below: u64,
    folded: u64,
    blanked: u64,
    masked_valid_code: u64,
    masked_weather: u64,
    sentinel_violations: u64,
    rays_with_ref: u64,
    rays_with_rho: u64,
    rays_with_zdr: u64,
    valid_on_rays_without_dual_pol: u64,
    low_rho: [u64; 3],
    low_rho_near: u64,
    low_rho_core_max: Option<f32>,
    rho_checked: u64,
}

/// Polar-grid holes: small non-echo regions fully enclosed by echo of at least HOLE_EDGE_DBZ.
fn holes(sweep: &Sweep) -> (u64, u64, u64) {
    let mut order: Vec<usize> = (0..sweep.rays.len())
        .filter(|i| sweep.rays[*i].moments[0].is_some())
        .collect();
    order.sort_by(|a, b| {
        sweep.rays[*a]
            .azimuth_deg
            .total_cmp(&sweep.rays[*b].azimuth_deg)
    });
    let n = order.len();
    let cell = |o: usize, g: usize| -> Option<(bool, f32, bool)> {
        let r = &sweep.rays[order[o]];
        let m = r.moments[0].as_ref()?;
        (g < m.values.len() && slant_m(m, g) <= MAX_GROUND_RANGE_M).then(|| {
            let raw = raw_gate(m, g, r.blanking);
            (valid(&raw), raw.dbz, raw.code == 1 || raw.blanked)
        })
    };
    let width = order
        .iter()
        .filter_map(|i| sweep.rays[*i].moments[0].as_ref().map(|m| m.values.len()))
        .max()
        .unwrap_or(0);
    let mut seen = vec![false; n * width];
    let (mut count, mut gates, mut masked) = (0, 0, 0);
    for o in 0..n {
        for g in 0..width {
            if seen[o * width + g] || !matches!(cell(o, g), Some((false, _, _))) {
                continue;
            }
            let mut queue = VecDeque::from([(o, g)]);
            seen[o * width + g] = true;
            let (mut size, mut enclosed, mut any_masked) = (0usize, true, false);
            while let Some((o, g)) = queue.pop_front() {
                size += 1;
                any_masked |= cell(o, g).is_some_and(|c| c.2);
                let mut next = vec![((o + 1) % n, g), ((o + n - 1) % n, g), (o, g + 1)];
                if g == 0 {
                    enclosed = false;
                } else {
                    next.push((o, g - 1));
                }
                for (no, ng) in next {
                    match cell(no, ng) {
                        None => enclosed = false,
                        Some((true, dbz, _)) => enclosed &= dbz >= HOLE_EDGE_DBZ,
                        Some((false, _, _)) => {
                            if !seen[no * width + ng] {
                                seen[no * width + ng] = true;
                                queue.push_back((no, ng));
                            }
                        }
                    }
                }
            }
            if enclosed && size <= HOLE_MAX_GATES {
                count += 1;
                gates += size as u64;
                masked += u64::from(any_masked);
            }
        }
    }
    (count, gates, masked)
}

fn gate_counts(sweep: &Sweep) -> (Counts, Option<(usize, usize, f32)>) {
    let mut c = Counts::default();
    let mut core: Option<(usize, usize, f32)> = None;
    for (ri, ray) in sweep.rays.iter().enumerate() {
        let Some(m) = &ray.moments[0] else { continue };
        c.rays_with_ref += 1;
        c.rays_with_rho += u64::from(ray.moments[1].is_some());
        c.rays_with_zdr += u64::from(ray.moments[2].is_some());
        let no_dual = ray.moments[1].is_none() || ray.moments[2].is_none();
        for g in 0..m.values.len() {
            let raw = raw_gate(m, g, ray.blanking);
            let ok = valid(&raw);
            let (v, q) = (m.values[g], m.qc[g]);
            let consistent = if ok {
                q == 0 && v.is_finite() && (v - raw.dbz).abs() < 1e-4
            } else {
                v.is_nan() && q != 0
            };
            c.sentinel_violations += u64::from(!consistent);
            let slant = slant_m(m, g);
            if slant > MAX_GROUND_RANGE_M {
                continue;
            }
            c.gates += 1;
            c.below += u64::from(q & BELOW != 0);
            c.folded += u64::from(q & FOLDED != 0);
            c.blanked += u64::from(q & BLANKED != 0);
            if raw.code >= 2 && raw.blanked {
                c.masked_valid_code += 1;
                c.masked_weather += u64::from(raw.dbz >= 10.0);
            }
            if !ok {
                continue;
            }
            c.valid += 1;
            c.valid_on_rays_without_dual_pol += u64::from(no_dual);
            if core.is_none_or(|(_, _, d)| raw.dbz > d) {
                core = Some((ri, g, raw.dbz));
            }
            if let Some(rho) = rho_at(sweep, ri, slant)
                && raw.dbz >= 5.0
            {
                c.rho_checked += 1;
                if rho < LOW_RHO {
                    let bin = if raw.dbz < 30.0 {
                        0
                    } else if raw.dbz < 45.0 {
                        1
                    } else {
                        2
                    };
                    c.low_rho[bin] += 1;
                    c.low_rho_near += u64::from(slant <= 40_000.0);
                    c.low_rho_core_max =
                        max_finite(c.low_rho_core_max.into_iter().chain([raw.dbz]));
                }
            }
        }
    }
    (c, core)
}

struct Palette {
    plte: Vec<u8>,
    trns: Vec<u8>,
    dbz_classes: usize,
}

const BG: [u8; 3] = [16, 18, 26];
const CLEAR: [u8; 3] = [44, 46, 58];
const RAW_BELOW: [u8; 3] = [70, 70, 84];
const RAW_FOLDED: [u8; 3] = [170, 0, 220];
const MASK_VALID: [u8; 3] = [205, 205, 205];
const MASK_BLANKED: [u8; 3] = [255, 140, 0];
const MASK_NO_RAY: [u8; 3] = [230, 30, 30];

/// Shipped classic reflectivity classes followed by the review colours above.
fn palette(model: &radar_tiler::ClassModel) -> Result<Palette> {
    let lut = model.lut("classic").context("classic palette")?;
    let mut plte = BG.to_vec();
    for rgba in &lut.rgba()[1..] {
        plte.extend_from_slice(&rgba[..3]);
    }
    let dbz_classes = plte.len() / 3;
    for c in [
        CLEAR,
        RAW_BELOW,
        RAW_FOLDED,
        MASK_VALID,
        MASK_BLANKED,
        MASK_NO_RAY,
    ] {
        plte.extend_from_slice(&c);
    }
    for i in 0..32u8 {
        let t = f32::from(i) / 31.0;
        plte.extend_from_slice(&[
            (255.0 * t) as u8,
            (255.0 * (1.0 - (2.0 * t - 1.0).abs())) as u8,
            (255.0 * (1.0 - t)) as u8,
        ]);
    }
    ensure!(plte.len() / 3 <= 256, "review palette too large");
    Ok(Palette {
        trns: vec![255; plte.len() / 3],
        plte,
        dbz_classes,
    })
}

impl Palette {
    fn extra(&self, i: usize) -> u8 {
        (self.dbz_classes + i) as u8
    }
    fn rho(&self, rho: f32) -> u8 {
        let t = ((rho - 0.2) / 0.85).clamp(0.0, 1.0);
        (self.dbz_classes + 6 + (t * 31.0) as usize) as u8
    }
}

fn write_png(path: &Path, px: &[u8], pal: &Palette) -> Result<()> {
    let png = radar_tiler::png::encode_indexed(px, PANEL_PX, PANEL_PX, &pal.plte, &pal.trns, 6)?;
    std::fs::write(path, png)?;
    Ok(())
}

struct GridResult {
    served_max: Option<f32>,
    sentinel_violations: u64,
    unserved_valid: u64,
    unserved_valid_without_dual_pol: u64,
    dual_pol_mismatch: u64,
    in_range_px: u64,
    no_ray_px: u64,
}

fn grid(
    sweep: &Sweep,
    full: &PolarSweep,
    stripped: &PolarSweep,
    model: &radar_tiler::ClassModel,
    pal: &Palette,
    out: Option<&Path>,
) -> Result<GridResult> {
    let site = sweep.site.context("missing site")?;
    let g = Grid {
        site: (f64::from(site.latitude_deg), f64::from(site.longitude_deg)),
    };
    let n = PANEL_PX * PANEL_PX;
    let (mut raw_dbz, mut qc_dbz) = (vec![f32::NAN; n], vec![f32::NAN; n]);
    let mut raw_extra: Vec<Option<u8>> = vec![None; n];
    let mut qc_extra: Vec<Option<u8>> = vec![None; n];
    let (mut mask, mut rho) = (vec![0u8; n], vec![0u8; n]);
    let mut r = GridResult {
        served_max: None,
        sentinel_violations: 0,
        unserved_valid: 0,
        unserved_valid_without_dual_pol: 0,
        dual_pol_mismatch: 0,
        in_range_px: 0,
        no_ray_px: 0,
    };
    for row in 0..PANEL_PX {
        for col in 0..PANEL_PX {
            let i = row * PANEL_PX + col;
            let (lat, lon) = g.lat_lon(row, col);
            let state = full.sample(lat, lon);
            r.dual_pol_mismatch += u64::from(stripped.sample(lat, lon) != state);
            let (ground, _) = radar_level2::polar::inverse_wgs84(g.site.0, g.site.1, lat, lon)?;
            if ground > MAX_GROUND_RANGE_M {
                continue;
            }
            r.in_range_px += 1;
            let Some(hit) = full.locate(lat, lon) else {
                r.no_ray_px += 1;
                mask[i] = pal.extra(5);
                ensure!(state == Gate::Missing, "sample without a ray");
                continue;
            };
            let ray = &sweep.rays[hit.ray];
            let m = ray.moments[0].as_ref().context("hit on ray without REF")?;
            let off = full_offset(sweep, hit.ray)?;
            let gate = hit.gate - off;
            let raw = raw_gate(m, gate, ray.blanking);
            let ok = valid(&raw);
            match state {
                Gate::Dbz(v) => {
                    r.sentinel_violations += u64::from(!ok || (v - raw.dbz).abs() > 1e-4);
                    r.served_max = max_finite(r.served_max.into_iter().chain([v]));
                    qc_dbz[i] = v;
                }
                Gate::Clear => qc_extra[i] = Some(pal.extra(0)),
                Gate::Missing => {}
            }
            if ok && !matches!(state, Gate::Dbz(_)) {
                r.unserved_valid += 1;
                r.unserved_valid_without_dual_pol +=
                    u64::from(ray.moments[1].is_none() || ray.moments[2].is_none());
            }
            match raw.code {
                0 => raw_extra[i] = Some(pal.extra(1)),
                1 => raw_extra[i] = Some(pal.extra(2)),
                _ => raw_dbz[i] = raw.dbz,
            }
            mask[i] = if raw.blanked {
                pal.extra(4)
            } else {
                match raw.code {
                    0 => pal.extra(1),
                    1 => pal.extra(2),
                    _ => pal.extra(3),
                }
            };
            if let Some(v) = rho_at(sweep, hit.ray, slant_m(m, gate))
                && ok
            {
                rho[i] = pal.rho(v);
            }
        }
    }
    if let Some(dir) = out {
        let classify = |dbz: &[f32], extra: &[Option<u8>]| -> Result<Vec<u8>> {
            let mut px = vec![0u8; n];
            model.classify_f32_into(dbz, &mut px)?;
            for (p, e) in px.iter_mut().zip(extra) {
                if let Some(e) = e {
                    *p = *e;
                }
            }
            Ok(px)
        };
        write_png(&dir.join("raw.png"), &classify(&raw_dbz, &raw_extra)?, pal)?;
        write_png(
            &dir.join("after-qc.png"),
            &classify(&qc_dbz, &qc_extra)?,
            pal,
        )?;
        write_png(&dir.join("mask.png"), &mask, pal)?;
        write_png(&dir.join("rho.png"), &rho, pal)?;
    }
    Ok(r)
}

/// `Hit::gate` indexes all REF gates in ray order; return the first index of `ray`.
fn full_offset(sweep: &Sweep, ray: usize) -> Result<usize> {
    Ok(sweep.rays[..ray]
        .iter()
        .map(|r| r.moments[0].as_ref().map_or(0, |m| m.values.len()))
        .sum())
}

/// Finds a point whose served gate is the raw core gate and returns the served value there.
fn served_core(sweep: &Sweep, s: &PolarSweep, core: (usize, usize, f32)) -> Result<Option<Value>> {
    let site = sweep.site.context("missing site")?;
    let site = (f64::from(site.latitude_deg), f64::from(site.longitude_deg));
    let ray = &sweep.rays[core.0];
    let m = ray.moments[0].as_ref().context("core ray")?;
    let slant = slant_m(m, core.1);
    let ground = slant * f64::from(ray.elevation_deg).to_radians().cos();
    let want = full_offset(sweep, core.0)? + core.1;
    let (clat, clon) = gate_lat_lon(site, f64::from(ray.azimuth_deg), ground);
    let step = 25.0 / 111_320.0;
    for dy in -60..=60 {
        for dx in -60..=60 {
            let lat = clat + f64::from(dy) * step;
            let lon = clon + f64::from(dx) * step / site.0.to_radians().cos();
            if s.locate(lat, lon).is_some_and(|h| h.gate == want) {
                let z9 = max_finite(tile_values(s, lat, lon, 9));
                let z12 = max_finite(tile_values(s, lat, lon, 12));
                let v = match s.sample(lat, lon) {
                    Gate::Dbz(v) => Some(v),
                    _ => None,
                };
                return Ok(Some(json!({
                    "lat": lat, "lon": lon, "served_dbz": v,
                    "tile_z9_max": z9, "tile_z12_max": z12,
                })));
            }
        }
    }
    Ok(None)
}

fn round(v: Option<f32>) -> Value {
    v.map_or(Value::Null, |v| json!((f64::from(v) * 10.0).round() / 10.0))
}

async fn review_sweep(
    sweep: &Sweep,
    stripped: &mut Sweep,
    source: &Source,
    model: &radar_tiler::ClassModel,
    pal: &Palette,
    out: Option<&Path>,
) -> Result<(Value, bool, bool)> {
    let full = served(sweep, source).await.context("served sweep")?;
    for r in &mut stripped.rays {
        r.moments[1] = None;
        r.moments[2] = None;
    }
    let lean = served(stripped, source).await;
    let (c, core) = gate_counts(sweep);
    let (hole_count, hole_gates, hole_masked) = holes(sweep);
    let (lean_ok, g) = match &lean {
        Ok(lean) => (true, grid(sweep, &full, lean, model, pal, out)?),
        Err(_) => (false, grid(sweep, &full, &full, model, pal, out)?),
    };
    let core_served = match core {
        Some(core) => served_core(sweep, &full, core)?,
        None => None,
    };
    let mut tiles_match = true;
    if let (Some(cs), Ok(lean)) = (&core_served, &lean) {
        let (lat, lon) = (
            cs["lat"].as_f64().unwrap_or(0.0),
            cs["lon"].as_f64().unwrap_or(0.0),
        );
        for z in [9, 12] {
            let a = tile_values(&full, lat, lon, z);
            let b = tile_values(lean, lat, lon, z);
            tiles_match &= a.iter().zip(&b).all(|(a, b)| a.to_bits() == b.to_bits());
        }
    }
    let sentinel_ok = c.sentinel_violations == 0 && g.sentinel_violations == 0;
    let dual_pol_ok = lean_ok
        && g.dual_pol_mismatch == 0
        && tiles_match
        && g.unserved_valid_without_dual_pol == 0;
    let elevation = sweep
        .rays
        .iter()
        .map(|r| f64::from(r.elevation_deg))
        .sum::<f64>()
        / sweep.rays.len().max(1) as f64;
    let raw_max = core.map(|c| c.2);
    let metrics = json!({
        "ordinal": sweep.ordinal,
        "elevation_number": sweep.elevation_number,
        "elevation_deg": (elevation * 100.0).round() / 100.0,
        "vcp": sweep.vcp,
        "scan_start": sweep.scan_start,
        "scan_end": sweep.scan_end,
        "rays": sweep.rays.len(),
        "missing_sectors_deg": sweep.missing_sectors_deg,
        "dual_pol": {"rays_with_ref": c.rays_with_ref, "rays_with_rho": c.rays_with_rho, "rays_with_zdr": c.rays_with_zdr,
            "valid_ref_gates_on_rays_without_dual_pol": c.valid_on_rays_without_dual_pol},
        "gates_within_150km": c.gates,
        "valid_gates": c.valid,
        "below_threshold_gates": c.below,
        "folded_gates": c.folded,
        "blanked_gates": c.blanked,
        "masked_valid_weather": {"valid_code_masked": c.masked_valid_code, "masked_ge_10dbz": c.masked_weather},
        "residual_clutter_proxy": {
            "rule": format!("retained REF >= 5 dBZ with RHO < {LOW_RHO}"),
            "rho_checked_gates": c.rho_checked,
            "ref_5_30": c.low_rho[0], "ref_30_45": c.low_rho[1], "ref_ge_45": c.low_rho[2],
            "within_40km": c.low_rho_near,
            "max_dbz": round(c.low_rho_core_max),
        },
        "core": {
            "raw_max_dbz": round(raw_max),
            "served_grid_max_dbz": round(g.served_max),
            "served_at_core_gate": core_served,
        },
        "holes": {"rule": format!("non-echo regions <= {HOLE_MAX_GATES} gates enclosed by REF >= {HOLE_EDGE_DBZ} dBZ"),
            "count": hole_count, "gates": hole_gates, "with_masked_gates": hole_masked},
        "grid": {"pixel_m": PANEL_M, "in_range_px": g.in_range_px, "no_ray_px": g.no_ray_px,
            "unserved_valid_px": g.unserved_valid},
        "hard_checks": {
            "no_sentinel_as_dbz": sentinel_ok,
            "sentinel_violations": {"gates": c.sentinel_violations, "served_px": g.sentinel_violations},
            "ref_kept_without_dual_pol": dual_pol_ok,
            "without_dual_pol": {"artifact_ok": lean_ok, "served_px_mismatch": g.dual_pol_mismatch,
                "core_tiles_identical": tiles_match, "unserved_valid_px_on_rays_without_dual_pol": g.unserved_valid_without_dual_pol},
        },
    });
    Ok((metrics, sentinel_ok, dual_pol_ok))
}

struct Run {
    id: String,
    station: String,
    path: PathBuf,
    source: Source,
    expect: Option<(usize, String)>,
    category: Vec<String>,
}

async fn review(run: &Run, out: &Path, panels: bool, screen: bool) -> Result<(Value, bool)> {
    let archive = std::fs::read(&run.path).with_context(|| run.path.display().to_string())?;
    if let Some((bytes, sha)) = &run.expect {
        ensure!(
            archive.len() == *bytes,
            "{}: byte count {}",
            run.id,
            archive.len()
        );
        let got = radar_level2::checksum(&archive);
        ensure!(&got == sha, "{}: sha256 {got}", run.id);
    }
    let volume = match run.path.to_str().and_then(|p| p.strip_suffix(".gz")) {
        Some(unpacked) => capped_read(
            std::fs::File::open(unpacked)?,
            radar_level2::COMPRESSED_LIMIT,
        )?,
        None => archive,
    };
    let dir = out.join(&run.id);
    std::fs::create_dir_all(&dir)?;
    let base = json!({"id": run.id, "category": run.category, "station": run.station,
        "key": run.source.key, "bytes": run.source.bytes, "sha256": run.source.sha256});
    let started = std::time::Instant::now();
    let decoded = decode(&run.station, &volume);
    let decode_ms = started.elapsed().as_millis();
    let (sweeps, finish_error) = match decoded {
        Ok(d) => d,
        Err(e) => (vec![], Some(format!("{e:#}"))),
    };
    let mut v = base;
    v["decode"] = json!({
        "archive_path_ok": finish_error.is_none() && !sweeps.is_empty(),
        "error": finish_error,
        "complete_lowest_sweeps": sweeps.len(),
        "decode_ms": decode_ms,
        "outcome": if finish_error.is_some() { "archive path rejects the volume; MRMS stays" } else { "published" },
    });
    if screen {
        v["sweeps"] = json!(sweeps.iter().map(|s| {
            let (c, core) = gate_counts(s);
            json!({"ordinal": s.ordinal, "rays": s.rays.len(), "missing_sectors_deg": s.missing_sectors_deg,
                "rays_with_rho": c.rays_with_rho, "folded": c.folded, "blanked": c.blanked,
                "masked_valid_code": c.masked_valid_code, "raw_max_dbz": round(core.map(|c| c.2))})
        }).collect::<Vec<_>>());
        std::fs::write(dir.join("metrics.json"), serde_json::to_vec_pretty(&v)?)?;
        return Ok((v, true));
    }
    let mut stripped = match decode(&run.station, &volume) {
        Ok((s, _)) => s,
        Err(_) => vec![],
    };
    let model = reflectivity_model()?;
    let pal = palette(&model)?;
    let mut all_ok = true;
    let mut per = vec![];
    let mut drawn: Vec<bool> = vec![];
    for (s, lean) in sweeps.iter().zip(stripped.iter_mut()) {
        let dual = s.rays.iter().any(|r| r.moments[1].is_some());
        let panel_dir =
            (panels && !drawn.contains(&dual)).then(|| dir.join(format!("sweep{}", s.ordinal)));
        if let Some(d) = &panel_dir {
            std::fs::create_dir_all(d)?;
            drawn.push(dual);
        }
        let (m, a, b) =
            review_sweep(s, lean, &run.source, &model, &pal, panel_dir.as_deref()).await?;
        all_ok &= a && b;
        per.push(m);
    }
    v["sweeps"] = json!(per);
    v["panels"] = json!(panels.then(|| dir.display().to_string()));
    std::fs::write(dir.join("metrics.json"), serde_json::to_vec_pretty(&v)?)?;
    Ok((v, all_ok))
}

fn runs(cli: &Cli) -> Result<Vec<Run>> {
    let mut runs = vec![];
    if let Some(path) = &cli.cases {
        let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(path)?)?;
        for c in cases {
            let (Some(file), Some(bytes), Some(sha), Some(url)) =
                (c.file, c.bytes, c.sha256, c.url)
            else {
                continue;
            };
            if cli
                .only
                .as_ref()
                .is_some_and(|o| !c.id.contains(o.as_str()))
            {
                continue;
            }
            let (bucket, key) = url
                .strip_prefix("https://")
                .and_then(|u| u.split_once(".s3.amazonaws.com/"))
                .with_context(|| format!("unexpected url {url}"))?;
            runs.push(Run {
                id: c.id,
                station: c.station,
                path: cli.archive.join(&file),
                source: Source {
                    bucket: bucket.into(),
                    key: key.into(),
                    sha256: sha.clone(),
                    bytes,
                },
                expect: Some((bytes, sha)),
                category: c.category,
            });
        }
    }
    for f in &cli.file {
        let (station, path) = f.split_once(':').context("--file STATION:PATH")?;
        let name = Path::new(path)
            .file_name()
            .context("file name")?
            .to_string_lossy()
            .into_owned();
        runs.push(Run {
            id: name.clone(),
            station: station.into(),
            path: path.into(),
            source: Source {
                bucket: "local".into(),
                key: name,
                sha256: String::new(),
                bytes: 0,
            },
            expect: None,
            category: vec![],
        });
    }
    Ok(runs)
}

/// One table row per reviewed sweep, for pasting into the review report.
fn markdown(cases: &[Value]) -> String {
    let mut t = String::from(
        "| Case | Sweep | El | RHO rays | Valid | Below | Folded | Blanked | Masked valid | Low-RHO 5-30 / 30-45 / >=45 (<=40 km) | Raw max | Served max | z9 / z12 at core | Holes (gates, masked) | No sentinel | REF w/o dual-pol |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for c in cases {
        if c["sweeps"].as_array().is_none_or(Vec::is_empty) {
            t += &format!(
                "| {} | none | | | | | | | | | | | | | | |\n",
                c["id"].as_str().unwrap_or_default()
            );
        }
        for s in c["sweeps"].as_array().into_iter().flatten() {
            let r = &s["residual_clutter_proxy"];
            let core = &s["core"]["served_at_core_gate"];
            let f = |v: &Value| match v.as_f64() {
                Some(x) => format!("{:.1}", x),
                None => "-".into(),
            };
            t += &format!(
                "| {} | {} | {} | {}/{} | {} | {} | {} | {} | {} | {} / {} / {} ({}) | {} | {} | {} / {} | {} ({}, {}) | {} | {} |\n",
                c["id"].as_str().unwrap_or_default(),
                s["ordinal"],
                s["elevation_deg"],
                s["dual_pol"]["rays_with_rho"],
                s["rays"],
                s["valid_gates"],
                s["below_threshold_gates"],
                s["folded_gates"],
                s["blanked_gates"],
                s["masked_valid_weather"]["valid_code_masked"],
                r["ref_5_30"],
                r["ref_30_45"],
                r["ref_ge_45"],
                r["within_40km"],
                f(&s["core"]["raw_max_dbz"]),
                f(&s["core"]["served_grid_max_dbz"]),
                f(&core["tile_z9_max"]),
                f(&core["tile_z12_max"]),
                s["holes"]["count"],
                s["holes"]["gates"],
                s["holes"]["with_masked_gates"],
                if s["hard_checks"]["no_sentinel_as_dbz"] == true {
                    "pass"
                } else {
                    "FAIL"
                },
                if s["hard_checks"]["ref_kept_without_dual_pol"] == true {
                    "pass"
                } else {
                    "FAIL"
                },
            );
        }
    }
    t
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let runs = runs(&cli)?;
    if runs.is_empty() {
        bail!("no cases to run");
    }
    std::fs::create_dir_all(&cli.out)?;
    let mut summary = vec![];
    let mut failed = vec![];
    for run in &runs {
        let (v, ok) = review(run, &cli.out, !cli.no_panels, cli.screen).await?;
        let s = &v["sweeps"][0];
        println!(
            "{} archive_ok={} raw_max={} served_max={} residual_low_rho={} holes={} sentinel_ok={} dual_pol_ok={}",
            run.id,
            v["decode"]["archive_path_ok"],
            s["core"]["raw_max_dbz"],
            s["core"]["served_grid_max_dbz"],
            s["residual_clutter_proxy"]["ref_5_30"],
            s["holes"]["count"],
            s["hard_checks"]["no_sentinel_as_dbz"],
            s["hard_checks"]["ref_kept_without_dual_pol"],
        );
        if !ok {
            failed.push(run.id.clone());
        }
        summary.push(v);
    }
    std::fs::write(cli.out.join("summary.md"), markdown(&summary))?;
    std::fs::write(
        cli.out.join("summary.json"),
        serde_json::to_vec_pretty(&json!({"cases": summary, "hard_check_failures": failed}))?,
    )?;
    if !failed.is_empty() {
        bail!("hard checks failed: {}", failed.join(", "));
    }
    println!("hard checks passed for {} case(s)", runs.len());
    Ok(())
}
