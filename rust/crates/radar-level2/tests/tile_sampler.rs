use radar_level2::decode::{BELOW, BLANKED, FOLDED, UNOBSERVED};
use radar_level2::polar::*;
use radar_tiler::{decode_indexed, geometry::lat_lon_to_tile};
use std::sync::atomic::AtomicBool;

const SITE: (f64, f64) = (42.89389, -85.54489);
type Cell = (f32, u8);

fn rays_at(
    azimuths: &[f32],
    spacing: f32,
    gates: usize,
    first: i16,
    el: impl Fn(usize) -> f32,
) -> Vec<RayInput> {
    azimuths
        .iter()
        .enumerate()
        .map(|(i, az)| RayInput {
            azimuth_deg: *az,
            elevation_deg: el(i),
            spacing_deg: spacing,
            first_gate_m: first,
            gate_m: 250,
            gates,
        })
        .collect()
}

fn sweep_at(
    site: (f64, f64),
    rays: &[RayInput],
    cell: impl Fn(usize, usize) -> Cell,
) -> PolarSweep {
    let (mut values, mut qc) = (vec![], vec![]);
    for (r, ray) in rays.iter().enumerate() {
        for g in 0..ray.gates {
            let (v, q) = cell(r, g);
            values.push(v);
            qc.push(q);
        }
    }
    PolarSweep::new("test".into(), site, rays, values, qc).unwrap()
}

fn sweep(rays: &[RayInput], cell: impl Fn(usize, usize) -> Cell) -> PolarSweep {
    sweep_at(SITE, rays, cell)
}

fn full_azimuths(step: f32) -> Vec<f32> {
    (0..(360.0 / step) as usize)
        .map(|i| i as f32 * step)
        .collect()
}

/// A point at `ground_m` and `bearing` from the site, refined with the geodesic inverse.
fn point(bearing: f64, ground_m: f64) -> (f64, f64) {
    let (mut lat, mut lon) = SITE;
    let (n, e) = (
        ground_m * bearing.to_radians().cos(),
        ground_m * bearing.to_radians().sin(),
    );
    lat += n / 111_200.0;
    lon += e / (111_320.0 * lat.to_radians().cos());
    for _ in 0..8 {
        let (s, a) = inverse_wgs84(SITE.0, SITE.1, lat, lon).unwrap();
        let (dn, de) = (n - s * a.to_radians().cos(), e - s * a.to_radians().sin());
        lat += dn / 111_200.0;
        lon += de / (111_320.0 * lat.to_radians().cos());
    }
    (lat, lon)
}

fn tile_for(lat: f64, lon: f64, z: u32) -> (u32, u32) {
    lat_lon_to_tile(lat, lon, z)
}

fn sampled(sw: &PolarSweep, z: u32, x: u32, y: u32) -> Vec<f32> {
    let mut out = vec![];
    assert!(sw.sample_tile(z, x, y, &AtomicBool::new(false), &mut out));
    out
}

fn rgba(png: &[u8]) -> Vec<[u8; 4]> {
    let d = decode_indexed(png).unwrap();
    assert_eq!((d.width, d.height), (256, 256));
    d.indices
        .iter()
        .map(|i| {
            let i = usize::from(*i);
            [
                d.plte[3 * i],
                d.plte[3 * i + 1],
                d.plte[3 * i + 2],
                d.trns.get(i).copied().unwrap_or(255),
            ]
        })
        .collect()
}

#[test]
fn fixture_gates_are_found_within_half_a_z13_pixel_with_wrap_and_variable_tilt() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../testdata/tile-geometry.json")).unwrap();
    let mut worst: f64 = 0.0;
    for p in fixtures.as_array().unwrap() {
        let n = |k: &str| p[k].as_f64().unwrap();
        let site = (n("site_lat"), n("site_lon"));
        let (tilt, bearing) = (n("tilt") as f32, n("bearing"));
        let wrap = !(1.0..=359.0).contains(&bearing);
        let azimuths: Vec<f32> = if wrap {
            vec![359.5, 0.5, 90.0, 180.0, 270.0]
        } else {
            vec![bearing as f32, 359.5]
        };
        let rays = rays_at(&azimuths, 1.0, 620, 0, |i| {
            if i == 0 || (wrap && i == 1) {
                tilt
            } else {
                tilt + 3.0
            }
        });
        let sw = sweep_at(site, &rays, |_, _| (20.0, 0));
        let hit = sw
            .locate(n("lat"), n("lon"))
            .unwrap_or_else(|| panic!("{p}"));
        let nearest = if bearing > 359.0 { 0 } else { 1 };
        assert!(
            hit.ray == if wrap { nearest } else { 0 } || bearing == 0.0,
            "{p}"
        );
        let error = (hit.slant_m - n("slant")).abs() / pixel_metres(13, n("lat"));
        worst = worst.max(error);
        let cells = n("slant") / 250.0;
        assert!(
            error <= 0.5
                && ((cells.fract() - 0.5).abs() < 1e-3 || hit.gate % 620 == cells.round() as usize),
            "{p}: {error}"
        );
    }
    println!("fixture locate worst_z13_pixel_error={worst}");
}

#[test]
fn each_ray_uses_its_own_tilt() {
    let rays = rays_at(
        &[0.0, 90.0],
        1.0,
        620,
        0,
        |i| if i == 0 { 0.5 } else { -0.5 },
    );
    let sw = sweep(&rays, |_, _| (20.0, 0));
    for (i, bearing) in [0.0, 90.0].into_iter().enumerate() {
        let (lat, lon) = point(bearing, 100_000.0);
        let (ground, _) = inverse_wgs84(SITE.0, SITE.1, lat, lon).unwrap();
        let expected = ground_to_slant(ground, if i == 0 { 0.5 } else { -0.5 }).unwrap();
        let hit = sw.locate(lat, lon).unwrap();
        assert_eq!(hit.ray, i);
        assert!((hit.slant_m - expected).abs() < 1e-6);
    }
}

#[test]
fn dense_field_has_no_seams_across_tiles_or_the_north_wrap() {
    let rays = rays_at(&full_azimuths(0.5), 0.5, 400, 2125, |_| 0.5);
    let sw = sweep(&rays, |_, _| (30.0, 0));
    for z in [9, 12] {
        let (cx, cy) = tile_for(SITE.0, SITE.1, z);
        let reach = if z == 9 { 1 } else { 3 };
        let mut checked = 0;
        for x in cx - reach..=cx + reach {
            for y in cy - reach..=cy + reach {
                let dbz = sampled(&sw, z, x, y);
                let world = 256.0 * f64::from(1u32 << z);
                for (i, v) in dbz.iter().enumerate() {
                    let (px, py) = (
                        f64::from(x) * 256.0 + (i % 256) as f64 + 0.5,
                        f64::from(y) * 256.0 + (i / 256) as f64 + 0.5,
                    );
                    let lon = px / world * 360.0 - 180.0;
                    let lat = (std::f64::consts::PI * (1.0 - 2.0 * py / world))
                        .sinh()
                        .atan()
                        .to_degrees();
                    let (ground, _) = inverse_wgs84(SITE.0, SITE.1, lat, lon).unwrap();
                    if (3_500.0..90_000.0).contains(&ground) {
                        checked += 1;
                        assert_eq!(*v, 30.0, "hole z{z} tile {x}/{y} pixel {i} ground {ground}");
                    }
                }
            }
        }
        assert!(checked > 20_000, "{checked}");
    }
}

#[test]
fn missing_sectors_are_never_stretched_and_wrap_is_continuous() {
    let azimuths: Vec<f32> = full_azimuths(0.5)
        .into_iter()
        .filter(|a| !(100.0..=130.0).contains(a))
        .collect();
    let rays = rays_at(&azimuths, 0.5, 400, 2125, |_| 0.5);
    let sw = sweep(&rays, |_, _| (30.0, 0));
    let at = |bearing: f64| {
        let (lat, lon) = point(bearing, 50_000.0);
        sw.sample(lat, lon)
    };
    assert_eq!(at(99.6), Gate::Dbz(30.0));
    assert_eq!(at(99.9), Gate::Missing);
    assert_eq!(at(115.0), Gate::Missing);
    assert_eq!(at(130.1), Gate::Missing);
    assert_eq!(at(130.45), Gate::Dbz(30.0));
    assert_eq!(at(359.9), Gate::Dbz(30.0));
    assert_eq!(at(0.1), Gate::Dbz(30.0));
    let (lat, lon) = point(115.0, 50_000.0);
    let (x, y) = tile_for(lat, lon, 12);
    assert!(sampled(&sw, 12, x, y).iter().any(|v| v.is_nan()));
}

#[test]
fn qc_holes_range_edges_and_the_150_km_mask() {
    let rays = rays_at(&[0.0], 360.0, 700, 2125, |_| 0.5);
    let sw = sweep(&rays, |_, g| match g {
        10 => (f32::NAN, BLANKED),
        11 => (f32::NAN, FOLDED),
        12 => (f32::NAN, UNOBSERVED),
        13 => (f32::NAN, BELOW),
        14 => (f32::NAN, BELOW | BLANKED),
        15 => (f32::NAN, 0),
        _ => (25.0, 0),
    });
    let at = |ground: f64| {
        let (lat, lon) = point(0.0, ground);
        sw.sample(lat, lon)
    };
    let slant = |g: usize| 2125.0 + 250.0 * g as f64;
    assert_eq!(at(slant(10)), Gate::Missing);
    assert_eq!(at(slant(11)), Gate::Missing);
    assert_eq!(at(slant(12)), Gate::Missing);
    assert_eq!(at(slant(13)), Gate::Clear);
    assert_eq!(at(slant(14)), Gate::Missing);
    assert_eq!(at(slant(15)), Gate::Missing);
    assert_eq!(at(slant(16)), Gate::Dbz(25.0));
    assert_eq!(at(1_900.0), Gate::Missing);
    assert_eq!(at(2_050.0), Gate::Dbz(25.0));
    assert_eq!(at(149_900.0), Gate::Dbz(25.0));
    assert_eq!(at(150_100.0), Gate::Missing);
    assert!(
        sw.locate(point(0.0, 120_000.0).0, point(0.0, 120_000.0).1)
            .is_some()
    );
    assert!(
        sw.locate(point(0.0, 150_100.0).0, point(0.0, 150_100.0).1)
            .is_none()
    );
}

fn reference(sw: &PolarSweep, z: u32, x: u32, y: u32, n: usize) -> Vec<f32> {
    let world = 256.0 * f64::from(1u32 << z);
    (0..256 * 256)
        .map(|i| {
            let (mut sum, mut count) = (0.0f64, 0);
            for sy in 0..n {
                for sx in 0..n {
                    let px = f64::from(x) * 256.0 + (i % 256) as f64 + (sx as f64 + 0.5) / n as f64;
                    let py = f64::from(y) * 256.0 + (i / 256) as f64 + (sy as f64 + 0.5) / n as f64;
                    let lon = px / world * 360.0 - 180.0;
                    let lat = (std::f64::consts::PI * (1.0 - 2.0 * py / world))
                        .sinh()
                        .atan()
                        .to_degrees();
                    match sw.sample(lat, lon) {
                        Gate::Dbz(v) => {
                            sum += 10f64.powf(f64::from(v) / 10.0);
                            count += 1;
                        }
                        Gate::Clear => count += 1,
                        Gate::Missing => {}
                    }
                }
            }
            if count > 0 && sum > 0.0 {
                (10.0 * (sum / f64::from(count)).log10()) as f32
            } else {
                f32::NAN
            }
        })
        .collect()
}

#[test]
fn thin_echoes_match_a_denser_linear_z_reference() {
    let rays = rays_at(&full_azimuths(0.5), 0.5, 400, 2125, |_| 0.5);
    let sw = sweep(&rays, |r, g| {
        if r == 90 && (100..300).contains(&g) {
            (35.0, 0)
        } else if g == 200 {
            (40.0, 0)
        } else {
            (f32::NAN, BELOW)
        }
    });
    for z in [8, 9] {
        let (x, y) = tile_for(SITE.0, SITE.1, z);
        let (ours, dense) = (sampled(&sw, z, x, y), reference(&sw, z, x, y, 8));
        let (mut error, mut n, mut worst) = (0.0f64, 0, 0.0f64);
        let (mut echo, mut missed, mut spurious, mut strongest_miss) = (0, 0, 0, f32::MIN);
        let (mut ours_z, mut dense_z) = (0.0f64, 0.0f64);
        let visible = |v: &f32| v.is_finite() && *v >= 5.0;
        let z_of = |v: &f32| {
            if v.is_finite() {
                10f64.powf(f64::from(*v) / 10.0)
            } else {
                0.0
            }
        };
        for (a, b) in ours.iter().zip(&dense) {
            (ours_z, dense_z) = (ours_z + z_of(a), dense_z + z_of(b));
            echo += usize::from(visible(b));
            if visible(b) && !visible(a) {
                missed += 1;
                strongest_miss = strongest_miss.max(*b);
            }
            spurious += usize::from(visible(a) && !visible(b));
            if a.is_finite() && b.is_finite() {
                let d = f64::from((a - b).abs());
                error += d;
                worst = worst.max(d);
                n += 1;
            }
        }
        let (mean, energy) = (error / f64::from(n), ours_z / dense_z);
        println!(
            "thin-echo z{z}: {echo} visible reference pixels, {missed} fringe pixels missed (strongest {strongest_miss:.1} dBZ), {spurious} spurious, mean abs error {mean:.3} dB, worst {worst:.3} dB over {n}, linear-Z energy ratio {energy:.3}"
        );
        // A 40 dBZ feature covering under a quarter of a pixel can fall between the 2x2 samples; anything stronger must be kept.
        assert!(
            echo > 30 && mean < 1.5 && worst < 8.0 && spurious == 0,
            "z{z}: mean {mean} worst {worst}"
        );
        assert!(
            strongest_miss <= 34.5 && (0.85..1.15).contains(&energy),
            "z{z}: strongest miss {strongest_miss} energy {energy}"
        );
    }
}

#[test]
fn hail_weak_and_negative_values_survive_minification() {
    let rays = rays_at(&full_azimuths(0.5), 0.5, 400, 2125, |_| 0.5);
    let core = sweep(&rays, |r, g| {
        if r == 180 && g == 200 {
            (65.0, 0)
        } else {
            (20.0, 0)
        }
    });
    let (lat, lon) = point(90.0, 2125.0 + 250.0 * 200.0);
    let model = reflectivity_model().unwrap();
    for z in [8, 9, 13] {
        let (x, y) = tile_for(lat, lon, z);
        let peak = sampled(&core, z, x, y)
            .into_iter()
            .filter(|v| v.is_finite())
            .fold(f32::MIN, f32::max);
        assert!(peak >= 55.0, "z{z} peak {peak}");
        let mut scratch = Scratch::default();
        scratch.dbz = sampled(&core, z, x, y);
        let strong = encode_tile(&model, "classic", &mut scratch).unwrap();
        let top = *decode_indexed(&strong.png)
            .unwrap()
            .indices
            .iter()
            .max()
            .unwrap();
        let mut hail = vec![55.0f32];
        let mut ref_classes = vec![0u8];
        model.classify_f32_into(&hail, &mut ref_classes).unwrap();
        hail.clear();
        assert!(
            top >= ref_classes[0],
            "z{z}: hail class {top} below 55 dBZ class {}",
            ref_classes[0]
        );
    }
    for value in [-10.0f32, -0.5, 4.999, 5.0] {
        let uniform = sweep(&rays, |_, _| (value, 0));
        for z in [8, 13] {
            let (lat, lon) = point(0.0, 10_000.0);
            let (x, y) = tile_for(lat, lon, z);
            let dbz = sampled(&uniform, z, x, y);
            assert!(
                dbz.iter().filter(|v| v.is_finite()).all(|v| *v == value)
                    && dbz.iter().any(|v| v.is_finite()),
                "{value} z{z}"
            );
        }
    }
}

#[test]
fn every_reflectivity_range_and_palette_matches_the_mrms_tables() {
    let model = reflectivity_model().unwrap();
    let rays = rays_at(&full_azimuths(0.5), 0.5, 400, 2125, |_| 0.5);
    let mut cases = vec![f32::MIN_POSITIVE, -32.0, 0.0, 4.999, 5.0, 75.0, 90.0];
    let tables: Vec<(String, serde_json::Value)> = ["classic", "muted", "vivid"]
        .iter()
        .map(|n| {
            (
                (*n).to_owned(),
                serde_json::from_str(
                    &std::fs::read_to_string(format!("../radar-tiler/assets/{n}.json")).unwrap(),
                )
                .unwrap(),
            )
        })
        .collect();
    for r in tables[0].1["reflectivity"]["ranges"].as_array().unwrap() {
        let (lo, hi) = (
            r["min"].as_f64().unwrap() as f32,
            r["max"].as_f64().unwrap() as f32,
        );
        cases.extend([lo, (lo + hi) / 2.0, hi.next_down(), hi]);
    }
    let (lat, lon) = point(0.0, 10_000.0);
    let mut checked = 0;
    for value in cases {
        let uniform = sweep(&rays, |_, _| (value, 0));
        for z in [13, 9] {
            let (x, y) = tile_for(lat, lon, z);
            let mut scratch = Scratch::default();
            scratch.dbz = sampled(&uniform, z, x, y);
            assert_eq!(scratch.dbz[128 * 256 + 128], value, "z{z} value changed");
            for (name, doc) in &tables {
                let png = encode_tile(&model, name, &mut scratch).unwrap().png;
                let pixel = rgba(&png)[128 * 256 + 128];
                let expected = doc["reflectivity"]["ranges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|r| {
                        f64::from(value) >= r["min"].as_f64().unwrap()
                            && f64::from(value) < r["max"].as_f64().unwrap()
                    });
                match expected {
                    Some(r) => {
                        let c: Vec<u8> = r["rgba"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_u64().unwrap() as u8)
                            .collect();
                        assert_eq!(pixel.to_vec(), c, "{name} {value} z{z}");
                    }
                    None => assert_eq!(pixel[3], 0, "{name} {value} z{z}"),
                }
                checked += 1;
            }
        }
    }
    println!("palette parity: {checked} value/palette/zoom checks against the MRMS tables");
}

#[test]
fn transparent_tiles_are_valid_pngs_and_scratch_stays_bounded() {
    let model = reflectivity_model().unwrap();
    let rays = rays_at(&full_azimuths(0.5), 0.5, 400, 2125, |_| 0.5);
    let clear = sweep(&rays, |_, _| (f32::NAN, BELOW));
    let mut scratch = Scratch::default();
    let cancel = AtomicBool::new(false);
    let (x, y) = tile_for(SITE.0, SITE.1, 9);
    let inside = clear
        .render_tile(9, x, y, &model, "classic", &cancel, &mut scratch)
        .unwrap();
    assert!(
        inside.transparent
            && decode_indexed(&inside.png)
                .unwrap()
                .indices
                .iter()
                .all(|i| *i == 0)
    );
    let (far_x, far_y) = tile_for(SITE.0 + 4.0, SITE.1, 8);
    let far = clear
        .render_tile(8, far_x, far_y, &model, "muted", &cancel, &mut scratch)
        .unwrap();
    assert!(far.transparent && !far.png.is_empty());
    let dense = sweep(&rays, |r, g| (((r + g) % 60) as f32 + 5.0, 0));
    let busy = dense
        .render_tile(9, x, y, &model, "vivid", &cancel, &mut scratch)
        .unwrap();
    assert!(!busy.transparent);
    println!(
        "dense tile png_bytes={} scratch_bytes={} sweep_bytes={}",
        busy.png.len(),
        scratch.bytes(),
        dense.bytes()
    );
    assert!(scratch.bytes() + busy.png.len() <= TILE_SCRATCH_BYTES);
    let stop = AtomicBool::new(true);
    assert!(
        dense
            .render_tile(9, x, y, &model, "vivid", &stop, &mut scratch)
            .is_err()
    );
}
