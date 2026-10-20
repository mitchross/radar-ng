//! Packed data tiles for the MRMS observation layers (`docs/reference/packed-tiles.md`).
//!
//! Each frame takes the next slot of its 30-minute window's bucket under `TILE_DIR/packed/<layer>/`.
//! The colored PNG pyramids are unchanged; packing is best effort and never fails a frame.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use chrono::{DateTime, Utc};
use radar_core::env::Paths;
use radar_core::manifest::canonical_timestamp;
use radar_core::manifest::packed::{
    PACKED_DIR, PackedBucket, PackedLayout, PackedTypes, observation_bucket_expired,
    observation_bucket_id, remove_packed_buckets,
};
use radar_geo::Grid;
use radar_tiler::packed::{
    PACKED_COLS, PACKED_MAX_SLOTS, PACKED_TILE_SIZE, PackFrame, PackOutcome, Quantizer,
    ValueSampler, pack_frames,
};
use radar_tiler::typed::{TYPE_CLASSES, TYPE_CODE_STRIDE};
use radar_tiler::{Overview, TypePlane};

/// 512-px zooms packed for MRMS: the 256-px pyramid's 4 to 7.
pub const MRMS_PACKED_ZOOMS: [u32; 4] = [3, 4, 5, 6];
/// Palette key of the MRMS values.
pub const COLOR_KEY: &str = "reflectivity";

/// `TILE_DIR/packed/<layer>`.
pub fn packed_layer_dir(paths: &Paths, layer: &str) -> PathBuf {
    paths.tile_dir.join(PACKED_DIR).join(layer)
}

/// The manifest `packed` object of an MRMS layer, without its buckets. `typed` layers store
/// precipitation-type codes.
pub fn mrms_layout(typed: bool) -> PackedLayout {
    let q = reflectivity();
    PackedLayout {
        tile_size: PACKED_TILE_SIZE as u32,
        min_zoom: MRMS_PACKED_ZOOMS[0],
        max_zoom: MRMS_PACKED_ZOOMS[MRMS_PACKED_ZOOMS.len() - 1],
        cols: PACKED_COLS as u32,
        max_slots: PACKED_MAX_SLOTS as u32,
        color_key: COLOR_KEY.to_string(),
        categorical: false,
        scale: q.scale,
        offset: q.offset,
        types: typed.then(|| PackedTypes {
            stride: u32::from(TYPE_CODE_STRIDE),
            classes: TYPE_CLASSES.iter().map(|c| c.to_string()).collect(),
        }),
    }
}

fn reflectivity() -> Quantizer {
    Quantizer::for_color_key(COLOR_KEY).expect("reflectivity is in the quantization table")
}

/// Pack one MRMS frame into its observation bucket. A `typed` layer stores typed codes; a frame
/// without `types` is all rain. Returns the bucket for the manifest, or `None` when the bucket is
/// already full. Takes the layer's packing lock; call it from a blocking thread.
pub fn pack_observation(
    paths: &Paths,
    layer: &str,
    timestamp: &str,
    grid: &Grid,
    typed: bool,
    types: Option<TypePlane>,
) -> anyhow::Result<Option<PackedBucket>> {
    let started = Instant::now();
    // Slots carry the manifest's canonical frame timestamp, so clients can match them.
    let slot = canonical_timestamp(timestamp)?;
    let id = observation_bucket_id(&slot).context("observation bucket id")?;
    let sampler = ValueSampler::new(
        grid,
        None,
        &MRMS_PACKED_ZOOMS,
        PACKED_TILE_SIZE,
        Some(Overview::Max),
    )?
    .with_types(types)?;
    let frames = [PackFrame {
        timestamp: &slot,
        sampler: &sampler,
    }];
    let layer_dir = packed_layer_dir(paths, layer);
    let quant = if typed {
        reflectivity().with_types()
    } else {
        reflectivity()
    };
    match pack_frames(&layer_dir, &id, &frames, quant)? {
        PackOutcome::Written {
            bucket,
            tiles_written,
        } => {
            tracing::info!(
                layer,
                bucket = %id,
                v = bucket.v,
                slots = bucket.slots.len(),
                tiles_written,
                duration_s = started.elapsed().as_secs_f64(),
                "packed_frame"
            );
            Ok(Some(PackedBucket {
                id,
                v: bucket.v,
                slots: bucket.slots,
            }))
        }
        PackOutcome::Full => {
            tracing::warn!(layer, bucket = %id, timestamp = %slot, "packed_bucket_full");
            Ok(None)
        }
    }
}

/// Remove observation buckets whose whole window is older than `cutoff`: manifest entries first,
/// then the directories. Returns the number of directories removed.
pub fn cleanup_observation_buckets(
    state_dir: &Path,
    paths: &Paths,
    layer: &str,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<u64> {
    remove_packed_buckets(state_dir, layer, |id| {
        observation_bucket_expired(id, cutoff)
    })?;
    let layer_dir = packed_layer_dir(paths, layer);
    let mut removed = 0u64;
    let entries = match std::fs::read_dir(&layer_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_dir() && observation_bucket_expired(name, cutoff) {
            std::fs::remove_dir_all(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::manifest::packed::{packed_buckets, upsert_packed_bucket};
    use radar_core::manifest::{AddOptions, add_frame, read_manifest};
    use radar_geo::GridGeo;

    fn storm_grid() -> Grid {
        let lats: Vec<f64> = (0..200).map(|i| 35.0 + 0.01 * f64::from(i)).collect();
        let lons: Vec<f64> = (0..200).map(|i| -98.0 + 0.01 * f64::from(i)).collect();
        Grid {
            height: 200,
            width: 200,
            values: vec![45.0; 200 * 200],
            geo: GridGeo::Regular { lats, lons },
        }
    }

    #[test]
    fn frames_fill_their_window_and_expire_manifest_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(
            dir.path().join("state"),
            dir.path().join("grids"),
            dir.path().join("tiles"),
        );
        let grid = storm_grid();
        let t0 = "2026-10-15T18:30:38+00:00";
        let t1 = "2026-10-15T18:32:40+00:00";
        let t2 = "2026-10-15T19:00:36+00:00";
        for t in [t0, t1, t2] {
            add_frame(&paths.state_dir, "radar", t, &AddOptions::default()).unwrap();
            let bucket = pack_observation(&paths, "radar", t, &grid, false, None)
                .unwrap()
                .unwrap();
            upsert_packed_bucket(&paths.state_dir, "radar", &mrms_layout(false), &bucket).unwrap();
        }
        let m = read_manifest(&paths.state_dir).unwrap();
        let buckets = packed_buckets(&m, "radar");
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].id, "2026-10-15T18:30:00Z");
        assert_eq!(buckets[0].v, 2);
        assert_eq!(buckets[0].slots, [t0, t1]);
        assert_eq!(buckets[1].id, "2026-10-15T19:00:00Z");
        assert_eq!(buckets[1].slots, [t2]);
        let layer_dir = packed_layer_dir(&paths, "radar");
        assert!(layer_dir.join("2026-10-15T18:30:00Z/6").is_dir());

        let cutoff = chrono::DateTime::parse_from_rfc3339("2026-10-15T19:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let removed =
            cleanup_observation_buckets(&paths.state_dir, &paths, "radar", cutoff).unwrap();
        assert_eq!(removed, 1);
        assert!(!layer_dir.join("2026-10-15T18:30:00Z").exists());
        assert!(layer_dir.join("2026-10-15T19:00:00Z").exists());
        let m = read_manifest(&paths.state_dir).unwrap();
        let ids: Vec<String> = packed_buckets(&m, "radar")
            .into_iter()
            .map(|b| b.id)
            .collect();
        assert_eq!(ids, ["2026-10-15T19:00:00Z"]);
    }
}
