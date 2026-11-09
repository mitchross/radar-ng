//! Tile planning, input cropping and playhead-to-object selection.

use chrono::{DateTime, Duration, Utc};
use radar_geo::{Grid, GridGeo};
use radar_tiler::geometry::{Sampling, SourceGeometry};
use thiserror::Error;

use crate::case::{CASE_TILE_SIZE, FAST_STALENESS_S, LayerSpec};

/// Source-cells of halo added around the tile footprint union before cropping.
pub const CROP_HALO_CELLS: usize = 2;

/// Plan failure.
#[derive(Debug, Error)]
pub enum PlanError {
    /// The grid has no regular lat/lon axes, so it cannot be cropped safely.
    #[error("replay cropping needs a regular lat/lon grid")]
    NotRegular,
    /// A tile does not overlap the source grid.
    #[error("tile {z}/{x}/{y} does not overlap the source grid")]
    NoCoverage {
        /// Zoom.
        z: u32,
        /// Tile column.
        x: u32,
        /// Tile row.
        y: u32,
    },
    /// The grid or the crop window is empty.
    #[error("empty grid or crop window")]
    Empty,
}

/// Every XYZ tile intersecting `[west, south, east, north]` at `zooms`, in zoom order.
pub fn case_tiles(bounds: &[f64; 4], zooms: &[u32]) -> Vec<(u32, u32, u32)> {
    let [west, south, east, north] = *bounds;
    let mut tiles = Vec::new();
    for &z in zooms {
        let (x0, y0) = radar_tiler::geometry::lat_lon_to_tile(north, west, z);
        let (x1, y1) = radar_tiler::geometry::lat_lon_to_tile(south, east, z);
        for x in x0..=x1 {
            for y in y0..=y1 {
                tiles.push((z, x, y));
            }
        }
    }
    tiles
}

/// Rows and columns required by `tiles`: the union of their bilinear footprints plus `halo` cells.
///
/// The union covers overview/max-pool footprints too, because a pooled tile spans every source
/// cell inside its pixel axes. Cropping to this window cannot change a rendered tile.
pub fn crop_window(
    src: &SourceGeometry,
    tiles: &[(u32, u32, u32)],
    tile_size: usize,
    halo: usize,
) -> Result<((usize, usize), (usize, usize)), PlanError> {
    let (mut r0, mut r1, mut c0, mut c1) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for &(z, x, y) in tiles {
        let geom = src
            .tile_geometry(Sampling::Bilinear, tile_size, z, x, y)
            .map_err(|_| PlanError::Empty)?
            .ok_or(PlanError::NoCoverage { z, x, y })?;
        r0 = r0.min(geom.row_bounds.0);
        r1 = r1.max(geom.row_bounds.1);
        c0 = c0.min(geom.col_bounds.0);
        c1 = c1.max(geom.col_bounds.1);
    }
    if r0 == usize::MAX || src.height() == 0 || src.width() == 0 {
        return Err(PlanError::Empty);
    }
    Ok((
        (r0.saturating_sub(halo), (r1 + halo).min(src.height() - 1)),
        (c0.saturating_sub(halo), (c1 + halo).min(src.width() - 1)),
    ))
}

/// Copy `window` (inclusive row and column ranges) out of a regular grid.
pub fn crop_grid(
    grid: &Grid,
    window: ((usize, usize), (usize, usize)),
) -> Result<Grid, PlanError> {
    let ((r0, r1), (c0, c1)) = window;
    if r1 < r0 || c1 < c0 || r1 >= grid.height || c1 >= grid.width {
        return Err(PlanError::Empty);
    }
    let height = r1 - r0 + 1;
    let width = c1 - c0 + 1;
    let mut values = Vec::with_capacity(height * width);
    for row in r0..=r1 {
        let start = row * grid.width + c0;
        values.extend_from_slice(&grid.values[start..start + width]);
    }
    let geo = match &grid.geo {
        GridGeo::Regular { lats, lons } => GridGeo::Regular {
            lats: lats[r0..=r1].to_vec(),
            lons: lons[c0..=c1].to_vec(),
        },
        GridGeo::Projected { .. } => return Err(PlanError::NotRegular),
    };
    Ok(Grid {
        width,
        height,
        values,
        geo,
    })
}

/// One frame to render: the layer object covering a playhead time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameChoice {
    /// Base-reflectivity playhead, or the QPE object's own valid time.
    pub playhead: DateTime<Utc>,
    /// Chosen S3 key.
    pub key: String,
    /// Chosen object's valid time.
    pub timestamp: DateTime<Utc>,
}

/// Pick the latest object at or before each `playheads` entry, within `max_staleness`.
///
/// Objects are `(valid time, key)` sorted ascending. A layer with no eligible object is skipped
/// and appears in the returned `missing` list.
pub fn choose_frames(
    playheads: &[DateTime<Utc>],
    objects: &[(DateTime<Utc>, String)],
    max_staleness: Duration,
) -> (Vec<FrameChoice>, Vec<DateTime<Utc>>) {
    let mut chosen = Vec::with_capacity(playheads.len());
    let mut missing = Vec::new();
    for &playhead in playheads {
        let found = objects
            .iter()
            .filter(|(t, _)| *t <= playhead && playhead - *t <= max_staleness)
            .max_by_key(|(t, _)| *t);
        match found {
            Some((t, key)) => chosen.push(FrameChoice {
                playhead,
                key: key.clone(),
                timestamp: *t,
            }),
            None => missing.push(playhead),
        }
    }
    (chosen, missing)
}

/// Staleness allowance for a layer: five minutes for fast layers, its own cadence for hourly QPE.
pub fn staleness_for(layer: &LayerSpec) -> Duration {
    if layer.cadence_s > FAST_STALENESS_S {
        Duration::seconds(layer.cadence_s)
    } else {
        Duration::seconds(FAST_STALENESS_S)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regular_grid(h: usize, w: usize, lat0: f64, lon0: f64, step: f64) -> Grid {
        let lats = (0..h).map(|i| lat0 + step * i as f64).collect();
        let lons = (0..w).map(|j| lon0 + step * j as f64).collect();
        let values = (0..h * w).map(|k| (k % 17) as f32).collect();
        Grid {
            width: w,
            height: h,
            values,
            geo: GridGeo::Regular { lats, lons },
        }
    }

    #[test]
    fn case_tiles_match_the_design_counts() {
        let rf = crate::case::find("rolling-fork-2023").unwrap();
        let tiles = case_tiles(&rf.bounds, &crate::case::CASE_ZOOMS);
        let counts: Vec<usize> = crate::case::CASE_ZOOMS
            .iter()
            .map(|z| tiles.iter().filter(|t| t.0 == *z).count())
            .collect();
        assert_eq!(counts, vec![2, 4, 4, 6]);
        let gf = crate::case::find("greenfield-2024").unwrap();
        let tiles = case_tiles(&gf.bounds, &crate::case::CASE_ZOOMS);
        let counts: Vec<usize> = crate::case::CASE_ZOOMS
            .iter()
            .map(|z| tiles.iter().filter(|t| t.0 == *z).count())
            .collect();
        assert_eq!(counts, vec![2, 2, 4, 9]);
    }

    #[test]
    fn crop_window_covers_every_planned_tile() {
        let grid = regular_grid(200, 260, 30.0, -100.0, 0.02);
        let src = SourceGeometry::from_grid(&grid).unwrap();
        let tiles = case_tiles(&[-97.0, 32.0, -94.0, 34.0], &[6, 7]);
        assert!(!tiles.is_empty());
        let ((r0, r1), (c0, c1)) = crop_window(&src, &tiles, CASE_TILE_SIZE, CROP_HALO_CELLS).unwrap();
        for &(z, x, y) in &tiles {
            let geom = src
                .tile_geometry(Sampling::Bilinear, CASE_TILE_SIZE, z, x, y)
                .unwrap()
                .unwrap();
            assert!(geom.row_bounds.0 >= r0 && geom.row_bounds.1 <= r1, "{z}/{x}/{y} rows");
            assert!(geom.col_bounds.0 >= c0 && geom.col_bounds.1 <= c1, "{z}/{x}/{y} cols");
        }
        let cropped = crop_grid(&grid, ((r0, r1), (c0, c1))).unwrap();
        assert_eq!(cropped.height, r1 - r0 + 1);
        assert_eq!(cropped.width, c1 - c0 + 1);
    }

    #[test]
    fn choose_frames_prefers_at_or_before_and_reports_stale_gaps() {
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let objects = vec![
            (at("2023-03-24T23:00:00Z"), "a".to_string()),
            (at("2023-03-24T23:02:20Z"), "b".to_string()),
        ];
        let playheads = [
            at("2023-03-24T23:00:15Z"),
            at("2023-03-24T23:02:15Z"),
            at("2023-03-24T23:20:00Z"),
        ];
        let (chosen, missing) = choose_frames(&playheads, &objects, Duration::seconds(300));
        assert_eq!(chosen.len(), 2);
        assert_eq!(chosen[0].key, "a");
        assert_eq!(chosen[1].key, "a", "next object is after the playhead");
        assert_eq!(missing, vec![playheads[2]]);
    }
}
