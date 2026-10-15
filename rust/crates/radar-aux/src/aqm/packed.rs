//! Packed data tiles for the AQM layers (`docs/packed-tiles.md`): one bucket per run and chunk.
//! Chunk `k` (messages `24k .. 24k + 24`) packs into part `k`, so a 72-hour run has three buckets:
//! `<run>`, `<run>.p1` and `<run>.p2` under `TILE_DIR/packed/<layer>`.
//!
//! A chunk packs its rendered frames in passes of three (one atlas block), after their PNG render.
//! Packing is best effort: a frame without a slot uses the raster path.

use std::path::{Path, PathBuf};

use radar_core::manifest::canonical_timestamp;
use radar_core::manifest::packed::{
    PACKED_DIR, PackedBucket, PackedLayout, run_bucket_id, run_layer_buckets, run_of_bucket,
};
use radar_geo::Grid;
use radar_tiler::packed::{
    PACKED_COLS, PACKED_MAX_SLOTS, PACKED_TILE_SIZE, PackFrame, PackOutcome, Quantizer,
    ValueSampler, list_buckets, pack_frames,
};

use super::{AqmError, CHUNK_MESSAGES, LayerSpec, parse_run_id};

/// 512-px zooms packed for AQM: the 256-px pyramid's 4 to 6.
pub const PACKED_ZOOMS: [u32; 3] = [3, 4, 5];
/// Frames sampled and merged per pack call: one atlas block.
pub(crate) const FRAMES_PER_PASS: usize = 3;

const _: () = assert!(
    CHUNK_MESSAGES <= PACKED_MAX_SLOTS,
    "a chunk fits one bucket"
);

/// `TILE_DIR/packed/<layer>`.
pub fn packed_layer_dir(tile_dir: &Path, layer: &str) -> PathBuf {
    tile_dir.join(PACKED_DIR).join(layer)
}

fn quantizer(spec: &LayerSpec) -> Result<Quantizer, AqmError> {
    Quantizer::for_color_key(spec.color_key)
        .ok_or_else(|| AqmError::Invalid(format!("no packed quantization for {}", spec.color_key)))
}

/// The manifest `packed` object of an AQM layer, without its buckets.
pub fn layout(spec: &LayerSpec) -> Result<PackedLayout, AqmError> {
    let q = quantizer(spec)?;
    Ok(PackedLayout {
        tile_size: PACKED_TILE_SIZE as u32,
        min_zoom: PACKED_ZOOMS[0],
        max_zoom: PACKED_ZOOMS[PACKED_ZOOMS.len() - 1],
        cols: PACKED_COLS as u32,
        max_slots: PACKED_MAX_SLOTS as u32,
        color_key: spec.color_key.to_string(),
        categorical: false,
        scale: q.scale,
        offset: q.offset,
        types: None,
    })
}

/// Bucket id of the chunk that starts at message `start`.
pub fn bucket_id(run_id: &str, start: usize) -> String {
    run_bucket_id(run_id, start / CHUNK_MESSAGES)
}

/// Pack `frames` (timestamp, north-first grid) of the chunk at `start` into its bucket. Takes the
/// layer's packing lock; call it from a blocking thread, outside any Rayon task.
pub(crate) fn pack_frames_of_chunk(
    tile_dir: &Path,
    spec: &LayerSpec,
    run_id: &str,
    start: usize,
    frames: &[(String, Grid)],
) -> Result<(), AqmError> {
    let quant = quantizer(spec)?;
    let slots: Vec<String> = frames
        .iter()
        .map(|(ts, _)| canonical_timestamp(ts))
        .collect::<Result<_, _>>()?;
    let samplers = frames
        .iter()
        .map(|(_, g)| ValueSampler::new(g, None, &PACKED_ZOOMS, PACKED_TILE_SIZE, None))
        .collect::<Result<Vec<_>, _>>()?;
    let pack: Vec<PackFrame<'_, '_>> = slots
        .iter()
        .zip(&samplers)
        .map(|(t, s)| PackFrame {
            timestamp: t,
            sampler: s,
        })
        .collect();
    let id = bucket_id(run_id, start);
    let layer_dir = packed_layer_dir(tile_dir, spec.name);
    match pack_frames(&layer_dir, &id, &pack, quant)? {
        PackOutcome::Written { .. } => Ok(()),
        PackOutcome::Full => Err(AqmError::Invalid(format!("packed bucket {id} is full"))),
    }
}

/// The buckets of run `run_id` on disk, with slots not in `timestamps` blanked.
pub fn run_buckets(
    tile_dir: &Path,
    layer: &str,
    run_id: &str,
    timestamps: &[String],
) -> Result<Vec<PackedBucket>, AqmError> {
    let buckets = list_buckets(&packed_layer_dir(tile_dir, layer))?
        .into_iter()
        .filter(|(id, _)| run_of_bucket(id) == run_id)
        .map(|(id, state)| PackedBucket {
            id,
            v: state.v,
            slots: state.slots,
        })
        .collect();
    Ok(run_layer_buckets(
        buckets,
        timestamps.iter().map(|t| (t.as_str(), run_id)),
    ))
}

/// Delete the layer's buckets of runs other than `current` whose issue time is before `cutoff`
/// (Unix seconds). Returns the number of directories removed.
pub(crate) fn cleanup_buckets(
    tile_dir: &Path,
    layer: &str,
    current: Option<&str>,
    cutoff: f64,
) -> Result<i64, AqmError> {
    let entries = match std::fs::read_dir(packed_layer_dir(tile_dir, layer)) {
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
        if !path.is_dir() || current == Some(run) {
            continue;
        }
        if parse_run_id(run).is_some_and(|t| (t.timestamp() as f64) < cutoff) {
            std::fs::remove_dir_all(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}
