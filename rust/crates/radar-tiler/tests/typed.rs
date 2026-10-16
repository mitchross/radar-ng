//! Precipitation-type coloring: rasters and packed codes pick the rain, snow or mix ramp per pixel.

use std::collections::BTreeSet;

use radar_tiler::packed::{PACKED_TILE_SIZE, PackOutcome, decode_rgb8, tile_path};
use radar_tiler::typed::{TYPE_MIX, TYPE_RAIN, TYPE_SNOW};
use radar_tiler::{
    FrameKind, FrameOptions, Grid, GridGeo, Overview, PackFrame, Quantizer, TypePlane,
    ValueSampler, decode_indexed, load_palette, pack_frames, render_frame_palettes,
};

/// 0.01 degree grid over lon -100..-96, lat 38..40 at 30 dBZ: rain west, snow middle, mix east.
fn grid_and_types() -> (Grid, TypePlane) {
    let lats: Vec<f64> = (0..200).map(|i| 38.0 + 0.01 * f64::from(i)).collect();
    let lons: Vec<f64> = (0..400).map(|i| -100.0 + 0.01 * f64::from(i)).collect();
    let (h, w) = (lats.len(), lons.len());
    let classes = (0..h * w)
        .map(|i| match i % w {
            c if c < 130 => TYPE_RAIN,
            c if c < 270 => TYPE_SNOW,
            _ => TYPE_MIX,
        })
        .collect();
    let grid = Grid {
        height: h,
        width: w,
        values: vec![32.5; h * w],
        geo: GridGeo::Regular { lats, lons },
    };
    (grid, TypePlane::new(classes).unwrap())
}

fn class_colors() -> [[u8; 4]; 3] {
    let table = load_palette("classic")
        .unwrap()
        .color_table("reflectivity")
        .unwrap();
    let raw = table.raw();
    let at = |ranges: &serde_json::Value| -> [u8; 4] {
        let r = ranges
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["min"].as_f64().unwrap() <= 32.0 && 32.0 < r["max"].as_f64().unwrap())
            .unwrap();
        let c: Vec<u8> = r["rgba"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        [c[0], c[1], c[2], c[3]]
    };
    [
        at(&raw["ranges"]),
        at(&raw["types"]["snow"]["ranges"]),
        at(&raw["types"]["mix"]["ranges"]),
    ]
}

#[test]
fn rasters_color_each_pixel_with_its_class_ramp() {
    let (grid, types) = grid_and_types();
    let table = load_palette("classic")
        .unwrap()
        .color_table("reflectivity")
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let options = FrameOptions {
        overview: Some(Overview::Max),
        types: Some(types),
        ..FrameOptions::default()
    };
    let root = tmp.path().join("classic");
    render_frame_palettes(
        &grid,
        &kind,
        &[("classic".into(), table)],
        &[("classic".into(), root.clone())],
        &[4, 7],
        &options,
    )
    .unwrap();
    let expected = class_colors();
    // z4 is max-pooled, z7 is bilinear: both routes see all three ramps and nothing else.
    for z in ["4", "7"] {
        let mut seen = BTreeSet::new();
        for x in std::fs::read_dir(root.join(z)).unwrap() {
            for y in std::fs::read_dir(x.unwrap().path()).unwrap() {
                let tile = decode_indexed(&std::fs::read(y.unwrap().path()).unwrap()).unwrap();
                for &i in &tile.indices {
                    let i = usize::from(i);
                    if i == 0 {
                        continue;
                    }
                    let rgb = &tile.plte[i * 3..i * 3 + 3];
                    seen.insert([rgb[0], rgb[1], rgb[2], tile.trns[i]]);
                }
            }
        }
        assert_eq!(seen, expected.into_iter().collect(), "zoom {z}");
    }
}

#[test]
fn packed_slots_hold_typed_codes() {
    let (grid, types) = grid_and_types();
    let sampler = ValueSampler::new(&grid, None, &[6], PACKED_TILE_SIZE, Some(Overview::Max))
        .unwrap()
        .with_types(Some(types))
        .unwrap();
    let quant = Quantizer::for_color_key("reflectivity")
        .unwrap()
        .with_types();
    let tmp = tempfile::tempdir().unwrap();
    let frame = PackFrame {
        timestamp: "2026-10-12T06:00:11+00:00",
        sampler: &sampler,
    };
    let PackOutcome::Written { bucket, .. } =
        pack_frames(tmp.path(), "b", &[frame], quant).unwrap()
    else {
        panic!("bucket full");
    };
    assert!(bucket.typed);
    let mut codes = BTreeSet::new();
    for (z, x, y) in sampler.tiles() {
        let path = tile_path(&tmp.path().join("b"), z, x, y);
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let (_, _, rgb) = decode_rgb8(&bytes).unwrap();
        codes.extend(rgb.chunks_exact(3).map(|p| p[0]).filter(|&c| c != 0));
    }
    // 32.5 dBZ is q = 13 (12 where bilinear rounding lands just below): rain q, snow 32 + q, mix 64 + q.
    let classes: BTreeSet<u8> = codes.iter().map(|c| c / 32).collect();
    assert_eq!(classes, BTreeSet::from([0, 1, 2]));
    assert!(codes.iter().all(|c| matches!(c % 32, 12 | 13)), "{codes:?}");
    // An untyped quantizer cannot add slots to a typed bucket.
    let plain = Quantizer::for_color_key("reflectivity").unwrap();
    let frame = PackFrame {
        timestamp: "2026-10-12T06:02:11+00:00",
        sampler: &sampler,
    };
    assert!(pack_frames(tmp.path(), "b", &[frame], plain).is_err());
}

#[test]
fn typed_codes_clamp_q_and_keep_zero() {
    let q = Quantizer::for_color_key("reflectivity")
        .unwrap()
        .with_types();
    assert_eq!(q.code(30.0, TYPE_RAIN), 12);
    assert_eq!(q.code(30.0, TYPE_SNOW), 44);
    assert_eq!(q.code(30.0, TYPE_MIX), 76);
    assert_eq!(q.code(90.0, TYPE_SNOW), 63); // q 36 clamps to 31
    assert_eq!(q.code(2.0, TYPE_MIX), 0);
    assert_eq!(q.code(f32::NAN, TYPE_SNOW), 0);
    let plain = Quantizer::for_color_key("reflectivity").unwrap();
    assert_eq!(plain.code(30.0, TYPE_SNOW), 12);
}
