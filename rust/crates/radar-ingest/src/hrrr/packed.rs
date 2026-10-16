//! Packed data tiles for the HRRR layers (`docs/packed-tiles.md`): one bucket per run, split into
//! parts of [`PACKED_MAX_SLOTS`] hours. Forecast hour `fhr` goes to part `(fhr - 1) / 24`, so an
//! 18 h run has one bucket and a 48 h run two.
//!
//! Each hour packs every rendered layer after its PNG render, on the calling blocking thread. Hours
//! run in parallel, so slots fill in completion order. Packing is best effort: a frame without a
//! slot uses the raster path. Publication lists the buckets of the runs the layer's frames come
//! from (see [`run_layer_buckets`]).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use chrono::DateTime;
use radar_core::env::Paths;
use radar_core::manifest::canonical_timestamp;
use radar_core::manifest::packed::{
    PACKED_DIR, PackedBucket, PackedLayout, run_bucket_id, run_layer_buckets, run_of_bucket,
};
use radar_core::time::parse_iso_as_utc;
use radar_geo::Grid;
use radar_grib::hrrr::parse_run_id;
use radar_tiler::packed::{
    PACKED_COLS, PACKED_MAX_SLOTS, PACKED_TILE_SIZE, PackFrame, PackOutcome, Quantizer,
    ValueSampler, list_buckets, pack_frames, read_bucket,
};

use super::state::layer_color_key;
use super::{run_init, valid_time};

/// 512-px zooms packed for HRRR: the 256-px pyramid's 4 to 6.
pub const PACKED_ZOOMS: [u32; 3] = [3, 4, 5];

/// `TILE_DIR/packed/<layer>`.
pub fn packed_layer_dir(paths: &Paths, layer: &str) -> PathBuf {
    paths.tile_dir.join(PACKED_DIR).join(layer)
}

/// The layer's quantization, or `None` when the layer is not packed (`precip-type` is categorical).
pub fn quantizer(layer: &str) -> Option<Quantizer> {
    Quantizer::for_color_key(layer_color_key(layer)?)
}

/// The manifest `packed` object of a packed HRRR layer, without its buckets.
pub fn layout(layer: &str) -> Option<PackedLayout> {
    let q = quantizer(layer)?;
    Some(PackedLayout {
        tile_size: PACKED_TILE_SIZE as u32,
        min_zoom: PACKED_ZOOMS[0],
        max_zoom: PACKED_ZOOMS[PACKED_ZOOMS.len() - 1],
        cols: PACKED_COLS as u32,
        max_slots: PACKED_MAX_SLOTS as u32,
        color_key: layer_color_key(layer)?.to_string(),
        categorical: false,
        scale: q.scale,
        offset: q.offset,
        types: None,
    })
}

/// Bucket id of a forecast hour: part `(fhr - 1) / 24` of the run.
pub fn bucket_id(run_id: &str, fhr: u32) -> String {
    run_bucket_id(run_id, fhr.saturating_sub(1) as usize / PACKED_MAX_SLOTS)
}

/// The forecast hour of `valid` in run `run_id`.
fn forecast_hour(run_id: &str, valid: &str) -> anyhow::Result<u32> {
    let init = run_init(run_id)?;
    let t = parse_iso_as_utc(valid).ok_or_else(|| anyhow::anyhow!("bad valid time {valid:?}"))?;
    let fhr = u32::try_from((t - init).num_hours())?;
    anyhow::ensure!(
        valid_time(init, fhr) == valid,
        "valid time {valid} is not a whole hour of run {run_id}"
    );
    Ok(fhr)
}

/// Pack one rendered hour of `layer` into its run bucket. A no-op for layers that are not packed.
/// Takes the layer's packing lock; call it from a blocking thread, outside any Rayon task.
pub fn pack_hour(
    paths: &Paths,
    layer: &str,
    run_id: &str,
    valid: &str,
    grid: &Grid,
) -> anyhow::Result<()> {
    let Some(quant) = quantizer(layer) else {
        return Ok(());
    };
    let started = Instant::now();
    let id = bucket_id(run_id, forecast_hour(run_id, valid)?);
    let slot = canonical_timestamp(valid)?;
    let sampler = ValueSampler::new(grid, None, &PACKED_ZOOMS, PACKED_TILE_SIZE, None)?;
    let frames = [PackFrame {
        timestamp: &slot,
        sampler: &sampler,
    }];
    match pack_frames(&packed_layer_dir(paths, layer), &id, &frames, quant)? {
        PackOutcome::Written {
            bucket,
            tiles_written,
        } => tracing::info!(
            layer,
            bucket = %id,
            v = bucket.v,
            slots = bucket.slots.len(),
            tiles_written,
            duration_s = started.elapsed().as_secs_f64(),
            "hrrr_packed"
        ),
        // Unreachable with one part per 24 hours; the frame keeps its PNG tiles.
        PackOutcome::Full => tracing::warn!(layer, bucket = %id, "packed_bucket_full"),
    }
    Ok(())
}

/// True when `layer` is not packed or its run bucket already holds `valid`.
pub fn has_slot(paths: &Paths, layer: &str, run_id: &str, valid: &str) -> bool {
    if quantizer(layer).is_none() {
        return true;
    }
    let (Ok(fhr), Ok(slot)) = (forecast_hour(run_id, valid), canonical_timestamp(valid)) else {
        return false;
    };
    let dir = packed_layer_dir(paths, layer).join(bucket_id(run_id, fhr));
    matches!(read_bucket(&dir), Ok(Some(b)) if b.slots.contains(&slot))
}

/// The buckets a layer lists for its frames, given as `(timestamp, run_id)`: the buckets on disk
/// of every run the frames come from, with other slots blanked.
pub fn run_buckets(
    paths: &Paths,
    layer: &str,
    frames: &[(String, String)],
) -> anyhow::Result<Vec<PackedBucket>> {
    let runs: BTreeSet<&str> = frames.iter().map(|(_, run)| run.as_str()).collect();
    let buckets = list_buckets(&packed_layer_dir(paths, layer))?
        .into_iter()
        .filter(|(id, _)| runs.contains(run_of_bucket(id)))
        .map(|(id, state)| PackedBucket {
            id,
            v: state.v,
            slots: state.slots,
        })
        .collect();
    Ok(run_layer_buckets(
        buckets,
        frames.iter().map(|(t, r)| (t.as_str(), r.as_str())),
    ))
}

/// Delete the layer's buckets of runs not in `live` whose init time is before `cutoff`, as the
/// PNG run directories are. Returns the number of directories removed.
pub fn cleanup_run_buckets(
    paths: &Paths,
    layer: &str,
    live: &BTreeSet<String>,
    cutoff: DateTime<chrono::Utc>,
) -> anyhow::Result<u64> {
    let entries = match std::fs::read_dir(packed_layer_dir(paths, layer)) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.into()),
    };
    let mut removed = 0;
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let run = run_of_bucket(name);
        if !path.is_dir() || live.contains(run) {
            continue;
        }
        if parse_run_id(run).is_some_and(|init| init < cutoff) {
            std::fs::remove_dir_all(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_geo::GridGeo;

    fn paths(dir: &std::path::Path) -> Paths {
        Paths::new(dir.join("state"), dir.join("grids"), dir.join("tiles"))
    }

    fn grid(value: f32) -> Grid {
        let lats: Vec<f64> = (0..100).map(|i| 35.0 + 0.02 * f64::from(i)).collect();
        let lons: Vec<f64> = (0..100).map(|i| -98.0 + 0.02 * f64::from(i)).collect();
        Grid {
            height: 100,
            width: 100,
            values: vec![value; 100 * 100],
            geo: GridGeo::Regular { lats, lons },
        }
    }

    #[test]
    fn layers_map_to_their_palette_quantization() {
        assert_eq!(layout("radar-hrrr").unwrap().color_key, "reflectivity");
        assert_eq!(layout("feels-like").unwrap().color_key, "temperature");
        let dew = layout("dewpoint").unwrap();
        assert_eq!((dew.scale, dew.offset, dew.max_zoom), (5.0, -80.0, 5));
        assert_eq!(layout("cape").unwrap().scale, 500.0);
        assert!(layout("precip-type").is_none(), "categorical");
        for layer in crate::config::HRRR_LAYERS {
            assert_eq!(layout(layer).is_some(), layer != "precip-type", "{layer}");
        }
    }

    #[test]
    fn hours_split_into_parts_of_24_and_cleanup_keeps_live_runs() {
        assert_eq!(bucket_id("20261015_12", 1), "20261015_12");
        assert_eq!(bucket_id("20261015_12", 24), "20261015_12");
        assert_eq!(bucket_id("20261015_12", 25), "20261015_12.p1");
        assert_eq!(bucket_id("20261015_12", 48), "20261015_12.p1");

        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let run = "20261015_12";
        let init = run_init(run).unwrap();
        let g = grid(60.0);
        for fhr in [1, 24, 25, 48] {
            pack_hour(&paths, "temperature", run, &valid_time(init, fhr), &g).unwrap();
        }
        // Not packed: no directory.
        pack_hour(&paths, "precip-type", run, &valid_time(init, 1), &g).unwrap();
        assert!(!packed_layer_dir(&paths, "precip-type").exists());
        assert!(has_slot(&paths, "temperature", run, &valid_time(init, 25)));
        assert!(!has_slot(&paths, "temperature", run, &valid_time(init, 2)));
        assert!(has_slot(&paths, "precip-type", run, &valid_time(init, 2)));
        let p0 = read_bucket(&packed_layer_dir(&paths, "temperature").join(run))
            .unwrap()
            .unwrap();
        assert_eq!(p0.slots.len(), 2);

        // The newer run shows hours 2 onwards of the older one: those slots are blanked.
        let newer = "20261015_13";
        pack_hour(&paths, "temperature", newer, &valid_time(init, 2), &g).unwrap();
        let frames: Vec<(String, String)> = vec![
            (valid_time(init, 1), run.into()),
            (valid_time(init, 2), newer.into()),
        ];
        let listed = run_buckets(&paths, "temperature", &frames).unwrap();
        let ids: Vec<&str> = listed.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, [run, newer]);
        assert_eq!(
            listed[0].slots,
            [canonical_timestamp(&frames[0].0).unwrap(), String::new()]
        );

        let live: BTreeSet<String> = [newer.to_string()].into();
        let cutoff = init + chrono::Duration::hours(1);
        assert_eq!(
            cleanup_run_buckets(&paths, "temperature", &live, cutoff).unwrap(),
            2
        );
        let left = list_buckets(&packed_layer_dir(&paths, "temperature")).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0, newer);
    }
}
