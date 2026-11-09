//! Output-tile allowlist and cropped-input parity (used by the replay importer).

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::{palette_table, png_tree};
use radar_tiler::geometry::{Sampling, SourceGeometry};
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
    vec![("classic".to_string(), palette_table("classic", "reflectivity"))]
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
    let g = grid(30.0, -100.0, 120, 120, 0.03);
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
fn cropped_input_matches_uncropped_boundary_tiles() {
    let g = grid(30.0, -100.0, 160, 160, 0.03);
    let src = SourceGeometry::from_grid(&g).unwrap();
    let all = src.tile_jobs(&[4, 5, 6]);
    // Keep one boundary tile per zoom: the last job of each zoom is the south-east corner.
    let mut wanted = Vec::new();
    for z in [4, 5, 6] {
        let last = all.iter().filter(|j| j.0 == z).next_back().copied().unwrap();
        wanted.push(last);
    }

    let full = tempfile::tempdir().unwrap();
    render(&g, full.path(), &[4, 5, 6], Some(wanted.clone()));

    let (rows, cols) = crop_window(&src, &wanted, 2);
    let crop = crop_grid(&g, rows, cols);
    let cropped = tempfile::tempdir().unwrap();
    render(&crop, cropped.path(), &[4, 5, 6], Some(wanted.clone()));

    let full_tiles = png_tree(&full.path().join("classic"));
    let crop_tiles = png_tree(&cropped.path().join("classic"));
    assert_eq!(full_tiles, crop_tiles, "cropped render changed the tile set");
    for rel in &full_tiles {
        let a = std::fs::read(full.path().join("classic").join(rel)).unwrap();
        let b = std::fs::read(cropped.path().join("classic").join(rel)).unwrap();
        assert_eq!(a, b, "{rel}: cropped input changed the tile bytes");
    }
}

/// Row and column range whose bilinear footprints cover `tiles`, plus `halo` cells.
fn crop_window(
    src: &SourceGeometry,
    tiles: &[(u32, u32, u32)],
    halo: usize,
) -> ((usize, usize), (usize, usize)) {
    let (mut r0, mut r1, mut c0, mut c1) = (usize::MAX, 0, usize::MAX, 0);
    for &(z, tx, ty) in tiles {
        let geom = src
            .tile_geometry(Sampling::Bilinear, TILE, z, tx, ty)
            .unwrap()
            .expect("tile overlaps the grid");
        r0 = r0.min(geom.row_bounds.0);
        r1 = r1.max(geom.row_bounds.1);
        c0 = c0.min(geom.col_bounds.0);
        c1 = c1.max(geom.col_bounds.1);
    }
    (
        (r0.saturating_sub(halo), (r1 + halo).min(src.height() - 1)),
        (c0.saturating_sub(halo), (c1 + halo).min(src.width() - 1)),
    )
}

fn crop_grid(g: &Grid, rows: (usize, usize), cols: (usize, usize)) -> Grid {
    let h = rows.1 - rows.0 + 1;
    let w = cols.1 - cols.0 + 1;
    let mut values = Vec::with_capacity(h * w);
    for r in rows.0..=rows.1 {
        values.extend_from_slice(&g.values[r * g.width + cols.0..r * g.width + cols.1 + 1]);
    }
    let geo = match &g.geo {
        GridGeo::Regular { lats, lons } => GridGeo::Regular {
            lats: lats[rows.0..=rows.1].to_vec(),
            lons: lons[cols.0..=cols.1].to_vec(),
        },
        other => other.clone(),
    };
    Grid {
        width: w,
        height: h,
        values,
        geo,
    }
}
