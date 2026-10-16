//! Packed data tiles for the nowcast layer (`docs/packed-tiles.md`): one bucket per run, its slots
//! the lead valid times, under `TILE_DIR/packed/nowcast/<anchor>`.
//!
//! Packing runs after the PNG render, on the calling (blocking) thread with the global Rayon pool,
//! never inside the nowcast render pool: [`pack_frames`] takes a file lock. It is best effort.

use std::path::PathBuf;
use std::time::Instant;

use chrono::{DateTime, Duration, Utc};
use radar_core::manifest::canonical_timestamp;
use radar_core::manifest::packed::{
    PACKED_DIR, PackedBucket, PackedLayout, run_bucket_id, run_of_bucket,
};
use radar_core::time::parse_iso_as_utc;
use radar_tiler::packed::{
    PACKED_COLS, PACKED_MAX_SLOTS, PACKED_TILE_SIZE, PackFrame, PackOutcome, Quantizer,
    ValueSampler, pack_frames,
};
use radar_tiler::{Grid, GridGeo, Overview};

use crate::config::NowcastConfig;
use crate::field::Field2;
use crate::render::DRY_BELOW_DBZ;
use crate::run::LAYER;

/// 512-px zooms packed for the nowcast: the 256-px pyramid's 4 to 6.
pub const PACKED_ZOOMS: [u32; 3] = [3, 4, 5];
/// Palette key of the nowcast values.
pub const COLOR_KEY: &str = "reflectivity";
/// Run buckets older than this are deleted (the tile sweep's nowcast retention).
pub const PACKED_RETENTION_MIN: i64 = 60;
/// Leads sampled and merged per pack call: one atlas block, so each tile is re-encoded once per
/// block and at most three lead grids are held at a time.
const LEADS_PER_PASS: usize = 3;

/// `TILE_DIR/packed/nowcast`.
pub fn packed_layer_dir(cfg: &NowcastConfig) -> PathBuf {
    cfg.paths.tile_dir.join(PACKED_DIR).join(LAYER)
}

/// The manifest `packed` object of the nowcast layer, without its buckets.
pub fn layout() -> PackedLayout {
    let q = reflectivity();
    PackedLayout {
        tile_size: PACKED_TILE_SIZE as u32,
        min_zoom: PACKED_ZOOMS[0],
        max_zoom: PACKED_ZOOMS[PACKED_ZOOMS.len() - 1],
        cols: PACKED_COLS as u32,
        max_slots: PACKED_MAX_SLOTS as u32,
        color_key: COLOR_KEY.to_string(),
        categorical: false,
        scale: q.scale,
        offset: q.offset,
    }
}

fn reflectivity() -> Quantizer {
    Quantizer::for_color_key(COLOR_KEY).expect("reflectivity is in the quantization table")
}

/// Pack every lead of run `anchor`. `timestamps[i]` is the valid time of `fields[i]`. Returns the
/// run's buckets in order (one per [`PACKED_MAX_SLOTS`] leads).
pub fn pack_run(
    cfg: &NowcastConfig,
    anchor: &str,
    lats: &[f64],
    lons: &[f64],
    fields: &[Field2<f32>],
    timestamps: &[String],
) -> anyhow::Result<Vec<PackedBucket>> {
    anyhow::ensure!(
        fields.len() == timestamps.len(),
        "packing {} fields with {} timestamps",
        fields.len(),
        timestamps.len()
    );
    let started = Instant::now();
    let layer_dir = packed_layer_dir(cfg);
    let quant = reflectivity();
    let slots: Vec<String> = timestamps
        .iter()
        .map(|t| canonical_timestamp(t))
        .collect::<Result<_, _>>()?;
    let mut buckets = Vec::new();
    for (part, (part_fields, part_slots)) in fields
        .chunks(PACKED_MAX_SLOTS)
        .zip(slots.chunks(PACKED_MAX_SLOTS))
        .enumerate()
    {
        let id = run_bucket_id(anchor, part);
        let mut last = None;
        for (pass_fields, pass_slots) in part_fields
            .chunks(LEADS_PER_PASS)
            .zip(part_slots.chunks(LEADS_PER_PASS))
        {
            let grids: Vec<Grid> = pass_fields
                .iter()
                .map(|f| lead_grid(f, lats, lons))
                .collect();
            let samplers = grids
                .iter()
                .map(|g| {
                    ValueSampler::new(
                        g,
                        None,
                        &PACKED_ZOOMS,
                        PACKED_TILE_SIZE,
                        Some(Overview::Max),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let frames: Vec<PackFrame<'_, '_>> = pass_slots
                .iter()
                .zip(&samplers)
                .map(|(t, s)| PackFrame {
                    timestamp: t,
                    sampler: s,
                })
                .collect();
            match pack_frames(&layer_dir, &id, &frames, quant)? {
                PackOutcome::Written { bucket, .. } => last = Some(bucket),
                PackOutcome::Full => anyhow::bail!("packed bucket {id} is full"),
            }
        }
        if let Some(bucket) = last {
            buckets.push(PackedBucket {
                id,
                v: bucket.v,
                slots: bucket.slots,
            });
        }
    }
    tracing::info!(
        anchor,
        buckets = buckets.len(),
        leads = fields.len(),
        duration_s = started.elapsed().as_secs_f64(),
        "nowcast_packed"
    );
    Ok(buckets)
}

/// The lead's values as the PNG render sees them: dry (below 5 dBZ) and missing cells are NaN.
fn lead_grid(field: &Field2<f32>, lats: &[f64], lons: &[f64]) -> Grid {
    Grid {
        width: field.width(),
        height: field.height(),
        values: field
            .as_slice()
            .iter()
            .map(|&v| {
                if v.is_nan() || v < DRY_BELOW_DBZ {
                    f32::NAN
                } else {
                    v
                }
            })
            .collect(),
        geo: GridGeo::Regular {
            lats: lats.to_vec(),
            lons: lons.to_vec(),
        },
    }
}

/// Delete run buckets older than [`PACKED_RETENTION_MIN`], except the current run's. The manifest
/// must already list only the current run. Returns the number of directories removed.
pub fn prune_run_buckets(
    cfg: &NowcastConfig,
    current: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<usize> {
    let cutoff = now - Duration::minutes(PACKED_RETENTION_MIN);
    let entries = match std::fs::read_dir(packed_layer_dir(cfg)) {
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
        if !path.is_dir() || run == current {
            continue;
        }
        if parse_iso_as_utc(run).is_some_and(|t| t < cutoff) {
            std::fs::remove_dir_all(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputs::linspace;
    use radar_core::env::Paths;
    use radar_tiler::packed::read_bucket;

    #[test]
    fn a_run_is_one_bucket_of_lead_slots_and_old_runs_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = NowcastConfig::with_paths(Paths::new(
            dir.path().join("state"),
            dir.path().join("grids"),
            dir.path().join("tiles"),
        ));
        let lats = linspace(45.0, 40.0, 64);
        let lons = linspace(-90.0, -80.0, 64);
        let fields = vec![Field2::filled(64, 64, 30.0f32); 12];
        let anchor = "2026-10-15T18:00:00+00:00";
        let timestamps: Vec<String> = (1..=12)
            .map(|i| format!("2026-10-15T{}:{:02}:00+00:00", 18 + i * 5 / 60, i * 5 % 60))
            .collect();
        let buckets = pack_run(&cfg, anchor, &lats, &lons, &fields, &timestamps).unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].id, anchor);
        assert_eq!(buckets[0].slots, timestamps);
        assert_eq!(buckets[0].v, 4, "12 leads in passes of 3");
        let on_disk = read_bucket(&packed_layer_dir(&cfg).join(anchor))
            .unwrap()
            .unwrap();
        assert_eq!(on_disk.slots, timestamps);

        let old = "2026-10-15T16:00:00+00:00";
        std::fs::create_dir_all(packed_layer_dir(&cfg).join(old).join("3")).unwrap();
        let now = parse_iso_as_utc("2026-10-15T18:02:00+00:00").unwrap();
        assert_eq!(prune_run_buckets(&cfg, anchor, now).unwrap(), 1);
        assert!(!packed_layer_dir(&cfg).join(old).exists());
        assert!(packed_layer_dir(&cfg).join(anchor).exists());
        // The current run is kept even once it is old.
        let later = now + Duration::hours(3);
        assert_eq!(prune_run_buckets(&cfg, anchor, later).unwrap(), 0);
    }
}
