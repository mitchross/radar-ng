//! Output-tile selection and publication identities for replay.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::{palette_table, png_tree};
use radar_tiler::geometry::SourceGeometry;
use radar_tiler::{FrameKind, FrameOptions, Grid, GridGeo, Overview, render_frame_palettes};

const TILE: usize = 64;

fn grid(lat0: f64, lon0: f64, h: usize, w: usize, step: f64) -> Grid {
    let lats: Vec<f64> = (0..h).map(|i| lat0 + step * i as f64).collect();
    let lons: Vec<f64> = (0..w).map(|j| lon0 + step * j as f64).collect();
    let values = (0..h * w)
        .map(|k| {
            let (r, c) = (k / w, k % w);
            if (20..60).contains(&r) && (20..60).contains(&c) {
                30.0 + (r as f32) * 0.2
            } else {
                f32::NAN
            }
        })
        .collect();
    Grid {
        width: w,
        height: h,
        values,
        geo: GridGeo::Regular { lats, lons },
    }
}

fn palettes() -> Vec<(String, radar_tiler::ColorTable)> {
    vec![(
        "classic".to_string(),
        palette_table("classic", "reflectivity"),
    )]
}

fn output(root: &Path) -> Vec<(String, PathBuf)> {
    vec![("classic".to_string(), root.join("classic"))]
}

fn render(grid: &Grid, root: &Path, zooms: &[u32], allow: Option<Vec<(u32, u32, u32)>>) {
    let options = FrameOptions {
        tile_size: TILE,
        source_id: "allowlist-test".into(),
        overview: Some(Overview::Max),
        tile_allowlist: allow,
        ..FrameOptions::default()
    };
    render_frame_palettes(
        grid,
        &FrameKind::Continuous {
            nodata_value: None,
            min_valid_weight: 1.0,
        },
        &palettes(),
        &output(root),
        zooms,
        &options,
    )
    .expect("render");
}

#[test]
fn allowlist_emits_only_the_requested_tiles() {
    let mut g = grid(30.0, -100.0, 120, 120, 0.03);
    g.values.fill(35.0);
    let all = SourceGeometry::from_grid(&g).unwrap().tile_jobs(&[6]);
    assert!(all.len() > 2, "grid must cover several tiles");
    let wanted: Vec<(u32, u32, u32)> = all.iter().copied().take(2).collect();

    let full = tempfile::tempdir().unwrap();
    render(&g, full.path(), &[6], None);
    assert_eq!(png_tree(&full.path().join("classic")), tile_set(&all));

    let limited = tempfile::tempdir().unwrap();
    render(&g, limited.path(), &[6], Some(wanted.clone()));
    assert_eq!(png_tree(&limited.path().join("classic")), tile_set(&wanted));
}

fn tile_set(tiles: &[(u32, u32, u32)]) -> BTreeSet<String> {
    tiles
        .iter()
        .map(|&(z, x, y)| format!("{z}/{x}/{y}.png"))
        .collect()
}

#[test]
fn changed_allowlist_cannot_reuse_an_existing_pyramid() {
    let g = grid(30.0, -100.0, 120, 120, 0.03);
    let all = SourceGeometry::from_grid(&g).unwrap().tile_jobs(&[6]);
    let tmp = tempfile::tempdir().unwrap();
    render(&g, tmp.path(), &[6], Some(all[..1].to_vec()));
    let options = FrameOptions {
        tile_size: TILE,
        source_id: "allowlist-test".into(),
        overview: Some(Overview::Max),
        tile_allowlist: Some(all),
        ..FrameOptions::default()
    };
    let result = render_frame_palettes(
        &g,
        &FrameKind::Continuous {
            nodata_value: None,
            min_valid_weight: 1.0,
        },
        &palettes(),
        &output(tmp.path()),
        &[6],
        &options,
    )
    .unwrap();
    assert_eq!(
        result.outcome("classic").unwrap().status,
        radar_tiler::PublishStatus::ExistingForeign
    );
}

#[test]
fn explicit_allowlist_keeps_transparent_boundary_tiles() {
    let mut g = grid(30.0, -100.0, 120, 120, 0.03);
    g.values.fill(f32::NAN);
    let all = SourceGeometry::from_grid(&g).unwrap().tile_jobs(&[6]);
    let wanted = all[..2].to_vec();
    let tmp = tempfile::tempdir().unwrap();
    render(&g, tmp.path(), &[6], Some(wanted.clone()));
    assert_eq!(png_tree(&tmp.path().join("classic")), tile_set(&wanted));
}
