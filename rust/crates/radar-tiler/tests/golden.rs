//! Pixel parity against the Python reference (spec-tiler section 18, fixtures in testdata/).
//!
//! Each golden tile is decoded with `decode_indexed`; class indices must match exactly, and the
//! set of emitted tiles must be identical. Byte parity with Pillow is not required.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use common::*;
use radar_tiler::{Crs, FrameKind, FrameOptions, Grid, GridGeo, Overview, render_frame_palettes};

fn outputs(root: &std::path::Path, names: &[&str]) -> Vec<(String, PathBuf)> {
    names
        .iter()
        .map(|n| (n.to_string(), root.join(n)))
        .collect()
}

#[test]
fn reflectivity_matches_python_indexed_tiles() {
    let dir = testdata("reflectivity");
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("meta.json")).unwrap()).unwrap();
    let (h, w) = (meta_usize(&meta, "height"), meta_usize(&meta, "width"));
    let grid = Grid {
        width: w,
        height: h,
        values: read_f32(&dir.join("data.f32")),
        geo: GridGeo::Regular {
            lats: read_f64(&dir.join("lats.f64")),
            lons: read_f64(&dir.join("lons.f64")),
        },
    };
    let names = ["classic", "muted", "vivid"];
    let palettes: Vec<_> = names
        .iter()
        .map(|n| (n.to_string(), palette_table(n, "reflectivity")))
        .collect();
    let tmp = tempfile::tempdir().unwrap();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let result = render_frame_palettes(
        &grid,
        &kind,
        &palettes,
        &outputs(tmp.path(), &names),
        &[4, 5, 6],
        &FrameOptions::default(),
    )
    .expect("render");
    assert_eq!(
        result.counts().iter().map(|(_, c)| *c).collect::<Vec<_>>(),
        vec![result.outcomes[0].1.tile_count; 3]
    );
    let mut total = 0;
    for n in names {
        total += assert_tiles_match_golden(
            &testdata(&format!("golden/reflectivity/{n}")),
            &tmp.path().join(n),
        );
    }
    assert!(total > 0);
}

#[test]
fn categorical_matches_python_nearest_tiles() {
    let dir = testdata("categorical");
    let (h, w) = (120usize, 240usize);
    let grid = Grid {
        width: w,
        height: h,
        values: read_i16(&dir.join("data.i16"))
            .into_iter()
            .map(f32::from)
            .collect(),
        geo: GridGeo::Regular {
            lats: read_f64(&dir.join("lats.f64")),
            lons: read_f64(&dir.join("lons.f64")),
        },
    };
    let names = ["classic", "muted", "vivid"];
    let palettes: Vec<_> = names
        .iter()
        .map(|n| (n.to_string(), palette_table(n, "precip_type")))
        .collect();
    let category_map: BTreeMap<i64, String> = [
        (1, "rain"),
        (2, "snow"),
        (3, "freezing_rain"),
        (4, "ice_pellets"),
    ]
    .iter()
    .map(|(k, v)| (*k, v.to_string()))
    .collect();
    let kind = FrameKind::Categorical { category_map };
    let tmp = tempfile::tempdir().unwrap();
    render_frame_palettes(
        &grid,
        &kind,
        &palettes,
        &outputs(tmp.path(), &names),
        &[5],
        &FrameOptions::default(),
    )
    .expect("render");
    for n in names {
        assert_tiles_match_golden(
            &testdata(&format!("golden/categorical/{n}")),
            &tmp.path().join(n),
        );
    }
}

#[test]
fn projected_3857_matches_python_tiles() {
    let dir = testdata("projected");
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("meta.json")).unwrap()).unwrap();
    let (h, w) = (meta_usize(&meta, "height"), meta_usize(&meta, "width"));
    let tile = meta_usize(&meta, "tile_size");
    let zooms: Vec<u32> = meta["zooms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|z| z.as_u64().unwrap() as u32)
        .collect();
    let table_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("table.json")).unwrap()).unwrap();
    let table = radar_tiler::ColorTable::from_value(&table_json).unwrap();
    let grid = Grid {
        width: w,
        height: h,
        values: read_f32(&dir.join("data.f32")),
        geo: GridGeo::Projected {
            crs: Crs::WebMercator,
            x: read_f64(&dir.join("source_x.f64")),
            y: read_f64(&dir.join("source_y.f64")),
            lat_mesh: read_f64(&dir.join("lat_mesh.f64")),
            lon_mesh: read_f64(&dir.join("lon_mesh.f64")),
        },
    };
    let tmp = tempfile::tempdir().unwrap();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let options = FrameOptions {
        tile_size: tile,
        ..FrameOptions::default()
    };
    render_frame_palettes(
        &grid,
        &kind,
        &[("p".into(), table)],
        &outputs(tmp.path(), &["p"]),
        &zooms,
        &options,
    )
    .expect("render");
    assert_tiles_match_golden(&testdata("golden/projected_3857/p"), &tmp.path().join("p"));
}

#[test]
fn overview_max_matches_python_footprint_tiles() {
    let dir = testdata("overview");
    let grid = Grid {
        width: 900,
        height: 600,
        values: read_f32(&dir.join("data.f32")),
        geo: GridGeo::Regular {
            lats: read_f64(&dir.join("lats.f64")),
            lons: read_f64(&dir.join("lons.f64")),
        },
    };
    let palettes = [(
        "classic".to_string(),
        palette_table("classic", "reflectivity"),
    )];
    let tmp = tempfile::tempdir().unwrap();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let options = FrameOptions {
        overview: Some(Overview::Max),
        ..FrameOptions::default()
    };
    render_frame_palettes(
        &grid,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4, 7],
        &options,
    )
    .expect("render");
    assert_tiles_match_golden(
        &testdata("golden/overview_max/classic"),
        &tmp.path().join("classic"),
    );
}
