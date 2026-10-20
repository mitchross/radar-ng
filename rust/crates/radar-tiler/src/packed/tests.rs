//! Packed tile format tests. The golden test documents what clients decode.

use super::*;
use crate::png::iter_chunks;
use radar_geo::GridGeo;

const REFL: Quantizer = Quantizer {
    scale: 2.5,
    offset: 0.0,
    floor: 5.0,
    typed: false,
};

/// A regular grid over lon -100..-90, lat 30..40 (0.1 degree), south-up, filled with `value`.
fn grid_filled(value: f32) -> Grid {
    let lats: Vec<f64> = (0..101).map(|i| 30.0 + 0.1 * f64::from(i)).collect();
    let lons: Vec<f64> = (0..101).map(|i| -100.0 + 0.1 * f64::from(i)).collect();
    Grid {
        height: lats.len(),
        width: lons.len(),
        values: vec![value; lats.len() * lons.len()],
        geo: GridGeo::Regular { lats, lons },
    }
}

fn pack(dir: &Path, id: &str, ts: &[&str], grids: &[&Grid], tile: usize) -> PackOutcome {
    let samplers: Vec<ValueSampler<'_>> = grids
        .iter()
        .map(|g| ValueSampler::new(g, None, &[3], tile, None).unwrap())
        .collect();
    let frames: Vec<PackFrame<'_, '_>> = ts
        .iter()
        .zip(&samplers)
        .map(|(t, s)| PackFrame {
            timestamp: t,
            sampler: s,
        })
        .collect();
    pack_frames(dir, id, &frames, REFL).unwrap()
}

fn written(outcome: PackOutcome) -> (BucketState, u64) {
    match outcome {
        PackOutcome::Written {
            bucket,
            tiles_written,
        } => (bucket, tiles_written),
        PackOutcome::Full => panic!("bucket unexpectedly full"),
    }
}

/// The only tile the test grid touches at zoom 3 with 512-px tiles: (3, 1, 3).
fn only_tile(bucket_dir: &Path) -> (usize, usize, Vec<u8>) {
    let tiles = existing_tiles(bucket_dir).unwrap();
    assert_eq!(tiles, vec![(3, 1, 3)]);
    decode_rgb8(&fs::read(tile_path(bucket_dir, 3, 1, 3)).unwrap()).unwrap()
}

/// Pixel of tile (3, 1, 3) at 512 px that lies inside the test grid (lat 35, lon -95).
fn inside_pixel() -> (usize, usize) {
    let n = 8.0f64;
    let fx = (-95.0f64 + 180.0) / 360.0 * n - 1.0;
    let lat = 35.0f64.to_radians();
    let fy = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n - 3.0;
    ((fx * 512.0) as usize, (fy * 512.0) as usize)
}

fn texel(atlas: &[u8], width: usize, x: usize, y: usize, channel: usize) -> u8 {
    atlas[(y * width + x) * 3 + channel]
}

#[test]
fn quantization_edges() {
    assert_eq!(REFL.quantize(f32::NAN), 0);
    assert_eq!(REFL.quantize(f32::NEG_INFINITY), 0);
    assert_eq!(REFL.quantize(f32::INFINITY), 255);
    assert_eq!(REFL.quantize(-999.0), 0);
    // Below the floor (5 dBZ, the lowest drawn band) is 0, whatever the code would be.
    assert_eq!(REFL.quantize(4.99), 0);
    assert_eq!(REFL.quantize(-20.0), 0);
    // Truncation keeps values in their 5 dBZ palette band: 44.9 stays in 40-45.
    assert_eq!(REFL.quantize(5.0), 2);
    assert_eq!(REFL.quantize(40.0), 16);
    assert_eq!(REFL.quantize(44.9), 17);
    assert_eq!(REFL.value(17), 42.5);
    assert_eq!(REFL.quantize(45.0), 18);
    assert_eq!(REFL.quantize(5000.0), 255);
    let cloud = Quantizer::for_color_key("cloud_cover").unwrap();
    assert_eq!(cloud.quantize(9.9), 0);
    assert_eq!(cloud.quantize(10.0), 2);
    // A floor below `offset + scale` lifts the values in between to code 1.
    let accum = Quantizer::for_color_key("precip_accum").unwrap();
    assert_eq!(accum.quantize(0.009), 0);
    assert_eq!(accum.quantize(0.011), 1);
    assert_eq!(accum.quantize(0.07), 1);
    assert_eq!(accum.quantize(0.1), 2);
    let wind = Quantizer::for_color_key("wind_speed").unwrap();
    assert_eq!(wind.quantize(0.0), 1);
    assert_eq!(wind.quantize(-0.1), 0);
    assert_eq!(Quantizer::for_color_key("reflectivity"), Some(REFL));
    assert_eq!(Quantizer::for_color_key("precip_type"), None);
    let temp = Quantizer::for_color_key("temperature").unwrap();
    assert_eq!(temp.quantize(-79.0), 1);
    assert_eq!(temp.quantize(175.0), 255);
}

/// Every palette band edge, as `(color key, edge)`, where a decoded value can land in another band
/// than the raw value (docs/reference/packed-tiles.md, "Quantization per palette key").
const INEXACT_EDGES: [(&str, f64); 11] = [
    ("humidity", 100.01),
    ("cloud_cover", 101.0),
    ("precip_accum", 20.0),
    ("pm25", 9.1),
    ("pm25", 20.0),
    ("pm25", 45.0),
    ("pm25", 90.0),
    ("pm25", 175.0),
    ("pm25", 500.0),
    ("pm25", 20000.0),
    ("ozone", 600.0),
];

/// The renderer's rule: below `no_data_below` is transparent, else the last range holding `v`.
fn band(table: &crate::palettes::ColorTable, v: f64) -> Option<[u8; 4]> {
    if v.is_nan() || v < table.no_data_below() {
        return None;
    }
    let ranges = table.ranges().unwrap_or_default();
    ranges
        .iter()
        .rev()
        .find(|r| v >= r.min && v < r.max)
        .map(|r| r.rgba)
}

#[test]
fn every_range_key_decodes_into_the_raw_band_except_the_listed_edges() {
    let mut checked = 0;
    for name in ["classic", "muted", "vivid"] {
        let palette = crate::load_palette(name).unwrap();
        for key in palette.color_keys() {
            let table = palette.color_table(key).unwrap();
            let Some(ranges) = table.ranges() else {
                assert_eq!(Quantizer::for_color_key(key), None, "{key} is categorical");
                continue;
            };
            let quant = Quantizer::for_color_key(key)
                .unwrap_or_else(|| panic!("{name}.{key} has ranges but no quantizer"));
            let mut edges: Vec<f64> = ranges.iter().flat_map(|r| [r.min, r.max]).collect();
            edges.sort_by(f64::total_cmp);
            let (lo, hi) = (edges[0] - 2.0 * quant.scale, edges[edges.len() - 1] * 1.01);
            let step = quant.scale.min(1.0) / 7.0;
            let mut inexact = std::collections::BTreeSet::new();
            let mut v = lo;
            while v <= hi {
                let raw = v as f32;
                let q = quant.quantize(raw);
                let decoded = (q != 0).then(|| quant.value(q));
                if band(&table, f64::from(raw)) != decoded.and_then(|d| band(&table, d)) {
                    let edge = edges.iter().rev().find(|&&e| e <= f64::from(raw)).copied();
                    inexact.insert(format!("{:?}", edge.unwrap_or(f64::NEG_INFINITY)));
                }
                v += step;
            }
            let expected: std::collections::BTreeSet<String> = INEXACT_EDGES
                .iter()
                .filter(|(k, _)| *k == key)
                .map(|(_, e)| format!("{e:?}"))
                .collect();
            assert_eq!(inexact, expected, "{name}.{key}");
            checked += 1;
        }
    }
    assert_eq!(checked, 33, "11 range keys in 3 palettes");
}

#[test]
fn slot_addressing() {
    let p = |s| slot_position(s, 512);
    assert_eq!(
        p(0),
        SlotPosition {
            block: 0,
            channel: 0,
            x0: 0,
            y0: 0
        }
    );
    assert_eq!(
        p(4),
        SlotPosition {
            block: 1,
            channel: 1,
            x0: 512,
            y0: 0
        }
    );
    assert_eq!(
        p(6),
        SlotPosition {
            block: 2,
            channel: 0,
            x0: 0,
            y0: 512
        }
    );
    assert_eq!(
        p(23),
        SlotPosition {
            block: 7,
            channel: 2,
            x0: 512,
            y0: 1536
        }
    );
    assert_eq!(atlas_dims(1, 512), (1024, 512));
    assert_eq!(atlas_dims(6, 512), (1024, 512));
    assert_eq!(atlas_dims(7, 512), (1024, 1024));
    assert_eq!(atlas_dims(PACKED_MAX_SLOTS, 512), (1024, 2048));
}

/// Golden format check: slot 4 of a 512-px bucket lands in block 1, channel G, at x = 512 + px.
#[test]
fn golden_slot_four_is_block_one_green() {
    let dir = tempfile::tempdir().unwrap();
    let empty = grid_filled(f32::NAN);
    let storm = grid_filled(40.0);
    let ts = ["t0", "t1", "t2", "t3", "t4"];
    let grids = [&empty, &empty, &empty, &empty, &storm];
    let (bucket, tiles) = written(pack(dir.path(), "2026-10-15T18:30:00Z", &ts, &grids, 512));
    assert_eq!(bucket.v, 1);
    assert_eq!(bucket.slots, ts);
    assert_eq!(tiles, 1);

    let bucket_dir = dir.path().join("2026-10-15T18:30:00Z");
    let bytes = fs::read(tile_path(&bucket_dir, 3, 1, 3)).unwrap();
    // IHDR: 1024 x 512, depth 8, color type 2 (RGB). No alpha, gamma or color-profile chunks.
    let chunks = iter_chunks(&bytes).unwrap();
    let types: Vec<&[u8; 4]> = chunks.iter().map(|(t, _)| t).collect();
    assert_eq!(types.first().copied(), Some(b"IHDR"));
    for banned in [b"gAMA", b"sRGB", b"iCCP", b"cHRM", b"tRNS", b"PLTE"] {
        assert!(
            !types.contains(&banned),
            "{:?}",
            std::str::from_utf8(banned)
        );
    }
    let ihdr = chunks[0].1;
    assert_eq!(u32::from_be_bytes(ihdr[0..4].try_into().unwrap()), 1024);
    assert_eq!(u32::from_be_bytes(ihdr[4..8].try_into().unwrap()), 512);
    assert_eq!((ihdr[8], ihdr[9]), (8, 2));

    let (w, h, atlas) = only_tile(&bucket_dir);
    assert_eq!((w, h), (1024, 512));
    let (px, py) = inside_pixel();
    assert_eq!(
        texel(&atlas, w, 512 + px, py, 1),
        16,
        "slot 4 = block 1, G (40 dBZ at 2.5 dBZ per code)"
    );
    assert_eq!(texel(&atlas, w, 512 + px, py, 0), 0, "slot 3 is empty");
    assert_eq!(texel(&atlas, w, 512 + px, py, 2), 0, "slot 5 is unused");
    assert!(
        atlas[..]
            .chunks(w * 3)
            .all(|row| row[..512 * 3].iter().all(|&b| b == 0))
    );
    // The bucket file is compact JSON, written last.
    let text = fs::read_to_string(bucket_dir.join(BUCKET_FILE)).unwrap();
    assert_eq!(text, r#"{"v":1,"slots":["t0","t1","t2","t3","t4"]}"#);
}

#[test]
fn second_slot_merges_and_keeps_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let a = grid_filled(20.0);
    let b = grid_filled(50.0);
    written(pack(dir.path(), "b", &["t0"], &[&a], 512));
    let (bucket, _) = written(pack(dir.path(), "b", &["t1"], &[&b], 512));
    assert_eq!(bucket.v, 2);
    assert_eq!(bucket.slots, ["t0", "t1"]);
    let (w, _, atlas) = only_tile(&dir.path().join("b"));
    let (px, py) = inside_pixel();
    assert_eq!(texel(&atlas, w, px, py, 0), REFL.quantize(20.0));
    assert_eq!(texel(&atlas, w, px, py, 1), REFL.quantize(50.0));
    assert_eq!(texel(&atlas, w, px, py, 2), 0);

    // Repacking an existing timestamp rewrites its slot in place and bumps v.
    let c = grid_filled(30.0);
    let (bucket, _) = written(pack(dir.path(), "b", &["t0"], &[&c], 512));
    assert_eq!(bucket.v, 3);
    assert_eq!(bucket.slots, ["t0", "t1"]);
    let (w, _, atlas) = only_tile(&dir.path().join("b"));
    assert_eq!(texel(&atlas, w, px, py, 0), REFL.quantize(30.0));
    assert_eq!(texel(&atlas, w, px, py, 1), REFL.quantize(50.0));
}

#[test]
fn all_zero_tiles_are_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let dry = grid_filled(-40.0); // below the table: q = 0 everywhere
    let (bucket, tiles) = written(pack(dir.path(), "b", &["t0"], &[&dry], 512));
    assert_eq!(tiles, 0);
    assert_eq!(bucket.slots, ["t0"]);
    assert!(existing_tiles(&dir.path().join("b")).unwrap().is_empty());
    assert!(dir.path().join("b").join(BUCKET_FILE).exists());

    // A rewrite that empties a tile removes the file.
    let wet = grid_filled(30.0);
    written(pack(dir.path(), "b", &["t1"], &[&wet], 512));
    assert_eq!(existing_tiles(&dir.path().join("b")).unwrap().len(), 1);
    written(pack(dir.path(), "b", &["t1"], &[&dry], 512));
    assert!(existing_tiles(&dir.path().join("b")).unwrap().is_empty());
}

#[test]
fn atlas_grows_for_untouched_tiles_and_bucket_fills_at_max_slots() {
    let dir = tempfile::tempdir().unwrap();
    let wet = grid_filled(30.0);
    let dry = grid_filled(f32::NAN);
    let tile = 16;
    written(pack(dir.path(), "b", &["s00"], &[&wet], tile));
    for s in 1..7 {
        let ts = format!("s{s:02}");
        let (bucket, _) = written(pack(dir.path(), "b", &[&ts], &[&dry], tile));
        assert_eq!(bucket.v, s as u64 + 1);
    }
    // Slot 6 starts a second block row: the tile that only has slot 0 is padded to it.
    let bytes = fs::read(tile_path(&dir.path().join("b"), 3, 1, 3)).unwrap();
    let (w, h, atlas) = decode_rgb8(&bytes).unwrap();
    assert_eq!((w, h), atlas_dims(7, tile));
    assert!(atlas.iter().step_by(3).any(|&q| q == REFL.quantize(30.0)));

    for s in 7..PACKED_MAX_SLOTS {
        written(pack(dir.path(), "b", &[&format!("s{s:02}")], &[&dry], tile));
    }
    let before = read_bucket(&dir.path().join("b")).unwrap().unwrap();
    assert_eq!(before.slots.len(), PACKED_MAX_SLOTS);
    let samplers = [ValueSampler::new(&dry, None, &[3], tile, None).unwrap()];
    let frames = [PackFrame {
        timestamp: "late",
        sampler: &samplers[0],
    }];
    assert_eq!(
        pack_frames(dir.path(), "b", &frames, REFL).unwrap(),
        PackOutcome::Full
    );
    assert_eq!(read_bucket(&dir.path().join("b")).unwrap().unwrap(), before);
}

#[test]
fn bucket_ids_must_be_one_safe_segment() {
    for bad in ["", ".", "..", ".packed.lock", "a/b", "a\\b"] {
        assert!(validate_bucket_id(bad).is_err(), "{bad:?}");
    }
    for good in [
        "2026-10-15T18:30:00Z",
        "2026-10-15T02:00:00+00:00",
        "2026-10-15T02:00:00+00:00.p1",
    ] {
        assert!(validate_bucket_id(good).is_ok(), "{good:?}");
    }
}

#[test]
fn values_follow_the_colored_tile_routes() {
    // A 0.01-degree grid is pooled at zoom 3 (512 px) and native at zoom 7: both give values.
    let lats: Vec<f64> = (0..400).map(|i| 30.0 + 0.01 * f64::from(i)).collect();
    let lons: Vec<f64> = (0..400).map(|i| -100.0 + 0.01 * f64::from(i)).collect();
    let mut values = vec![f32::NAN; 400 * 400];
    values[200 * 400 + 200] = 60.0; // a one-cell storm survives max pooling
    let grid = Grid {
        height: 400,
        width: 400,
        values,
        geo: GridGeo::Regular { lats, lons },
    };
    let tiles = sample_tiles(&grid, None, &[3], 512, Some(Overview::Max)).unwrap();
    let max = tiles
        .iter()
        .flat_map(|(_, v)| v.iter().copied())
        .fold(f32::NAN, f32::max);
    assert_eq!(max, 60.0);
}
