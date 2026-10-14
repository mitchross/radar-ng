//! CPU work of one chunk: decode each message, orient it, render the tiles for every palette, and write
//! grid dumps for the frames that were recorded. Runs on the blocking pool.

use std::borrow::Cow;
use std::path::PathBuf;

use radar_core::grid_dump::{GridConfig, GridInput, write_grid};
use radar_core::time::isoformat_utc;
use radar_geo::{Grid, GridGeo};
use radar_tiler::palettes::ColorTable;
use radar_tiler::{FrameKind, FrameOptions, render_frame_palettes};
use tracing::warn;

use super::grib::{split_messages, valid_time};
use super::{AqmError, LayerSpec, ZOOM_LEVELS};

/// Everything one chunk needs, owned, so it can move onto the blocking pool.
pub(crate) struct ChunkJob {
    /// Layer spec (color key, unit, name).
    pub layer: &'static LayerSpec,
    /// Run id, `YYYYMMDD_CC`.
    pub run_id: String,
    /// First message index (zero-based), inclusive.
    pub start: usize,
    /// End message index, exclusive.
    pub end: usize,
    /// Path of the cached GRIB2 file.
    pub grib_path: PathBuf,
    /// Palettes that carry the layer's color key, in `PALETTES` order.
    pub tables: Vec<(String, ColorTable)>,
    /// `TILE_DIR`.
    pub tile_dir: PathBuf,
    /// Grid dump settings.
    pub grid: GridConfig,
}

/// Renders messages `start..end` and returns the timestamps of the frames that were recorded, in order.
///
/// A frame is recorded only when every palette rendered it. A frame that is all transparent (no tiles)
/// is skipped and gets no grid dump.
pub(crate) fn render_chunk_blocking(job: &ChunkJob) -> Result<Vec<String>, AqmError> {
    let bytes = std::fs::read(&job.grib_path)?;
    let messages = split_messages(&bytes)?;
    if messages.len() < job.end {
        return Err(AqmError::MissingMessage {
            found: messages.len(),
            needed: job.end,
        });
    }
    let mut rendered = Vec::new();
    for index in job.start..job.end {
        let msg = &bytes[messages[index].clone()];
        let valid = valid_time(msg).map_err(|e| match e {
            AqmError::Message { detail, .. } => AqmError::Message { index, detail },
            other => other,
        })?;
        let ts = isoformat_utc(&valid);
        let grid = radar_grib::decode_first_message(msg)?.grid;
        if render_frame(job, &ts, &grid)?.is_empty() {
            continue;
        }
        rendered.push(ts.clone());
        dump_grid(job, &ts, &grid);
    }
    Ok(rendered)
}

/// Renders one frame into every palette. Returns the palettes rendered, or nothing unless all rendered.
fn render_frame(job: &ChunkJob, ts: &str, grid: &Grid) -> Result<Vec<String>, AqmError> {
    if job.tables.is_empty() {
        return Ok(Vec::new());
    }
    let layer_dir = job.tile_dir.join(job.layer.name);
    let tile_path = super::frame_path(&job.run_id, ts);
    let out_dirs: Vec<(String, PathBuf)> = job
        .tables
        .iter()
        .map(|(name, _)| (name.clone(), layer_dir.join(name).join(&tile_path)))
        .collect();
    // The renderer wants north-first (Python `_write_palette_tiles`). Flip a copy only when needed.
    let flipped = orientation_copy(grid);
    let render_grid = flipped.as_ref().unwrap_or(grid);
    let options = FrameOptions {
        source_id: format!("aqm:{}:{}", job.layer.name, tile_path),
        publication_lock_root: Some(layer_dir),
        ..FrameOptions::default()
    };
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let result = render_frame_palettes(
        render_grid,
        &kind,
        &job.tables,
        &out_dirs,
        &ZOOM_LEVELS,
        &options,
    )?;
    let rendered: Vec<String> = result
        .rendered_palettes()
        .into_iter()
        .map(str::to_string)
        .collect();
    if rendered.len() != job.tables.len() {
        return Ok(Vec::new());
    }
    Ok(rendered)
}

/// The grid flipped so that its rows run north first (spec 5.7), or `None` when no flip is needed.
///
/// A regular grid is flipped when `lats[0] > lats[-1]`. A projected grid is flipped when
/// `source_y[0] > source_y[-1]`.
pub(crate) fn orientation_copy(grid: &Grid) -> Option<Grid> {
    let needs_flip = match &grid.geo {
        GridGeo::Regular { lats, .. } => lats.len() >= 2 && lats[0] > lats[lats.len() - 1],
        GridGeo::Projected { y, .. } => y.len() >= 2 && y[0] > y[y.len() - 1],
    };
    needs_flip.then(|| {
        let mut flipped = grid.clone();
        flipped.flip_rows();
        flipped
    })
}

/// Writes the grid dump for one recorded frame. Errors are logged and swallowed, as in Python
/// (`_safe_grid_dump`): a failed dump never fails the frame.
fn dump_grid(job: &ChunkJob, ts: &str, grid: &Grid) {
    let (lats, lons) = dump_axes(grid);
    // Spec D-7 (decided): longitudes are stored ascending. Python keeps the source order. radar-core's
    // writer only sorts values above 180, so the reorder happens here.
    let (data, lons) = ascending_columns(&grid.values, grid.width, lons);
    let input = GridInput::new(&data, &lats, &lons, job.layer.unit);
    if let Err(err) = write_grid(&job.grid, job.layer.name, ts, &input) {
        warn!(layer = job.layer.name, ts, error = %err, "grid_dump_failed");
    }
}

/// The 1-D axes for a grid dump (Python `_grid_dump_axes`): for a projected grid, the latitude column at
/// `w // 2` and the longitude row at `h // 2`. The axes are taken from the un-flipped grid.
pub(crate) fn dump_axes(grid: &Grid) -> (Vec<f64>, Vec<f64>) {
    match &grid.geo {
        GridGeo::Regular { lats, lons } => (lats.clone(), lons.clone()),
        GridGeo::Projected {
            lat_mesh, lon_mesh, ..
        } => {
            let (h, w) = (grid.height, grid.width);
            let col = w / 2;
            let row = h / 2;
            let lats = (0..h).map(|r| lat_mesh[r * w + col]).collect();
            let lons = (0..w).map(|c| lon_mesh[row * w + c]).collect();
            (lats, lons)
        }
    }
}

/// Reorders the columns of a row-major `h x w` grid so that `lons` ascends. Returns the grid unchanged
/// (borrowed) when `lons` already ascends.
pub(crate) fn ascending_columns(
    values: &[f32],
    w: usize,
    lons: Vec<f64>,
) -> (Cow<'_, [f32]>, Vec<f64>) {
    if lons.windows(2).all(|p| p[0] <= p[1]) {
        return (Cow::Borrowed(values), lons);
    }
    let mut order: Vec<usize> = (0..w).collect();
    order.sort_by(|&a, &b| lons[a].total_cmp(&lons[b]));
    let sorted_lons: Vec<f64> = order.iter().map(|&i| lons[i]).collect();
    let h = values.len().checked_div(w).unwrap_or(0);
    let mut out = Vec::with_capacity(values.len());
    for r in 0..h {
        for &c in &order {
            out.push(values[r * w + c]);
        }
    }
    (Cow::Owned(out), sorted_lons)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascending_columns_keeps_ascending_input_borrowed() {
        let values = [1.0f32, 2.0, 3.0, 4.0];
        let (out, lons) = ascending_columns(&values, 2, vec![-90.0, -89.0]);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(lons, vec![-90.0, -89.0]);
    }

    #[test]
    fn ascending_columns_sorts_descending_longitudes() {
        // 2 rows x 3 columns, lons decreasing.
        let values = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        let (out, lons) = ascending_columns(&values, 3, vec![-80.0, -81.0, -82.0]);
        assert_eq!(lons, vec![-82.0, -81.0, -80.0]);
        assert_eq!(&out[..], &[3.0, 2.0, 1.0, 6.0, 5.0, 4.0]);
    }

    #[test]
    fn orientation_flips_only_descending_latitudes() {
        let flat = |lats: Vec<f64>| Grid {
            width: 1,
            height: lats.len(),
            values: (0..lats.len()).map(|i| i as f32).collect(),
            geo: GridGeo::Regular {
                lats,
                lons: vec![-90.0],
            },
        };
        assert!(orientation_copy(&flat(vec![40.0, 41.0, 42.0])).is_none());
        let flipped =
            orientation_copy(&flat(vec![42.0, 41.0, 40.0])).expect("descending needs a flip");
        // Flipping a descending grid makes it ascending.
        match flipped.geo {
            GridGeo::Regular { lats, .. } => assert_eq!(lats, vec![40.0, 41.0, 42.0]),
            _ => panic!("regular"),
        }
        assert_eq!(flipped.values, vec![2.0, 1.0, 0.0]);
    }
}
