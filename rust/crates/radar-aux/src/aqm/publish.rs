//! `aqm_publish_run`: commits each layer to the manifest as one atomic replacement of its frame list.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use radar_core::manifest::packed::{PACKED_KEY, packed_metadata, remove_packed_buckets};
use radar_core::manifest::{FrameSpec, ReplaceOptions, read_manifest, replace_layer};
use radar_core::time::parse_iso_as_utc;
use radar_types::payloads::airquality::AqmChunkResult;
use serde_json::{Map, Value, json};

use super::{
    AQM_LAYER_SPECS, AqmError, FORECAST_MESSAGES, LayerSpec, MAX_ZOOM, PaletteKeys,
    SPATIAL_RESOLUTION_KM, frame_path, parse_run_id,
};
use radar_core::time::isoformat_utc;

/// Publishes every complete layer (Python `_publish_run_sync`). Returns the published layer names.
///
/// A layer needs exactly 72 distinct rendered timestamps and at least one palette that carries its color
/// key. Otherwise it is skipped and the manifest keeps the previous run for that layer. With
/// `packed` (`TILE_DIR`, when `PACKED_TILES` is on), the layer also lists the run's packed buckets
/// in the same manifest write.
pub(crate) fn publish_run(
    state_dir: &Path,
    packed: Option<&Path>,
    run_id: &str,
    results: &[AqmChunkResult],
    palettes: &[PaletteKeys],
) -> Result<Vec<String>, AqmError> {
    let run_dt = parse_run_id(run_id).ok_or_else(|| {
        AqmError::Invalid(format!("bad AQM run id {run_id:?}, expected YYYYMMDD_CC"))
    })?;
    let run_issued_at = isoformat_utc(&run_dt);
    let mut published = Vec::new();

    for spec in AQM_LAYER_SPECS.iter() {
        let timestamps: BTreeSet<String> = results
            .iter()
            .filter(|r| r.layer == spec.name)
            .flat_map(|r| r.rendered_timestamps.iter().cloned())
            .collect();
        if timestamps.len() != FORECAST_MESSAGES {
            continue;
        }
        let mut layer_palettes: Vec<String> = palettes
            .iter()
            .filter(|p| p.keys.contains(spec.color_key))
            .map(|p| p.name.clone())
            .collect();
        if layer_palettes.is_empty() {
            continue;
        }
        layer_palettes.sort();

        let frames: Vec<FrameSpec> = timestamps
            .iter()
            .map(|ts| -> Result<FrameSpec, AqmError> {
                let valid = parse_iso_as_utc(ts)
                    .ok_or_else(|| AqmError::Invalid(format!("bad frame timestamp {ts:?}")))?;
                Ok(FrameSpec {
                    timestamp: ts.clone(),
                    path: Some(frame_path(run_id, ts)),
                    palettes: Vec::new(),
                    extra: frame_extra(run_id, &run_issued_at, lead_minutes(&valid, &run_dt)),
                })
            })
            .collect::<Result<_, _>>()?;

        let mut metadata = Map::new();
        metadata.insert("title".into(), json!(spec.title));
        metadata.insert("kind".into(), json!("model_guidance"));
        metadata.insert("run_id".into(), json!(run_id));
        metadata.insert("complete".into(), json!(true));

        let timestamp_list: Vec<String> = timestamps.iter().cloned().collect();
        match packed.and_then(|tile_dir| packed_value(tile_dir, spec, run_id, &timestamp_list)) {
            Some(value) => {
                metadata.insert(PACKED_KEY.into(), value);
            }
            // No buckets: drop stale ones first, so the new frames never map to another run's slots.
            None if read_manifest(state_dir)?
                .layers
                .get(spec.name)
                .is_some_and(|l| l.extra.contains_key(PACKED_KEY)) =>
            {
                remove_packed_buckets(state_dir, spec.name, |_| true)?;
            }
            None => {}
        }
        replace_layer(
            state_dir,
            spec.name,
            &timestamp_list,
            &ReplaceOptions {
                palettes: layer_palettes,
                frames,
                layer_metadata: Some(metadata),
            },
        )?;
        published.push(spec.name.to_string());
    }
    Ok(published)
}

/// The layer's `packed` object, or `None` (no buckets, or an error, which is logged: the frames then
/// use their PNG tiles).
fn packed_value(
    tile_dir: &Path,
    spec: &LayerSpec,
    run_id: &str,
    timestamps: &[String],
) -> Option<Value> {
    let listed = super::packed::layout(spec).and_then(|layout| {
        let buckets = super::packed::run_buckets(tile_dir, spec.name, run_id, timestamps)?;
        Ok(packed_metadata(&layout, buckets))
    });
    match listed {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(layer = spec.name, error = %err, "packed_manifest_failed");
            None
        }
    }
}

/// `lead_minutes = int((ts - run_dt).total_seconds() // 60)`: floor division, so negative leads floor down.
fn lead_minutes(valid: &DateTime<Utc>, run_dt: &DateTime<Utc>) -> i64 {
    (*valid - *run_dt).num_seconds().div_euclid(60)
}

/// The extra frame keys the Python writes (spec 5.12).
fn frame_extra(run_id: &str, run_issued_at: &str, lead: i64) -> Map<String, Value> {
    let mut extra = Map::new();
    extra.insert("source".into(), json!("aqm"));
    extra.insert("kind".into(), json!("model_guidance"));
    extra.insert("issued_at".into(), json!(run_issued_at));
    extra.insert("run_id".into(), json!(run_id));
    extra.insert("lead_minutes".into(), json!(lead));
    extra.insert("spatial_resolution_km".into(), json!(SPATIAL_RESOLUTION_KM));
    extra.insert("max_zoom".into(), json!(MAX_ZOOM));
    extra
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::manifest::read_manifest;
    use radar_core::time::isoformat_utc;
    use tempfile::TempDir;

    /// `n` hourly timestamps for the layer, starting at the run time (12:00 on 2026-07-16).
    fn hourly(layer: &str, n: usize) -> AqmChunkResult {
        let start =
            chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 7, 16, 12, 0, 0).unwrap();
        let rendered_timestamps = (0..n)
            .map(|i| isoformat_utc(&(start + chrono::Duration::hours(i as i64))))
            .collect();
        AqmChunkResult {
            layer: layer.to_string(),
            start_msg: 0,
            rendered_timestamps,
        }
    }

    fn keys(name: &str, color_keys: &[&str]) -> PaletteKeys {
        PaletteKeys {
            name: name.to_string(),
            keys: color_keys.iter().map(|k| k.to_string()).collect(),
        }
    }

    #[test]
    fn layer_palettes_are_the_ones_carrying_its_color_key() {
        let dir = TempDir::new().unwrap();
        let palettes = [
            keys("vivid", &["pm25"]),
            keys("classic", &["pm25", "ozone"]),
            keys("muted", &["ozone"]),
        ];
        let results = [hourly("air-quality", 72)];
        let published = publish_run(dir.path(), None, "20260716_12", &results, &palettes).unwrap();
        assert_eq!(published, vec!["air-quality".to_string()]);
        let layer = &read_manifest(dir.path()).unwrap().layers["air-quality"];
        assert_eq!(
            layer.palettes,
            vec!["classic".to_string(), "vivid".to_string()]
        );
    }

    #[test]
    fn layer_without_a_palette_for_its_key_is_not_published() {
        let dir = TempDir::new().unwrap();
        let palettes = [keys("classic", &["ozone"])];
        let published = publish_run(
            dir.path(),
            None,
            "20260716_12",
            &[hourly("air-quality", 72)],
            &palettes,
        )
        .unwrap();
        assert!(published.is_empty());
        assert!(read_manifest(dir.path()).unwrap().layers.is_empty());
    }

    #[test]
    fn frames_are_sorted_and_lead_minutes_are_relative_to_the_run() {
        let dir = TempDir::new().unwrap();
        let palettes = [keys("classic", &["pm25"])];
        // Results arrive out of order; the manifest must still be sorted.
        let mut reversed = hourly("air-quality", 72);
        reversed.rendered_timestamps.reverse();
        publish_run(dir.path(), None, "20260716_12", &[reversed], &palettes).unwrap();
        let layer = &read_manifest(dir.path()).unwrap().layers["air-quality"];
        assert_eq!(layer.timestamps.len(), 72);
        assert!(
            layer.timestamps.windows(2).all(|w| w[0] < w[1]),
            "timestamps sorted and unique"
        );
        assert_eq!(layer.frames[0].extra["lead_minutes"], 0);
        assert_eq!(layer.frames[71].extra["lead_minutes"], 71 * 60);
    }
}
