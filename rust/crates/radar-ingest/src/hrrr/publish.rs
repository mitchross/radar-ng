//! Coherent HRRR publication to the manifest (spec §11.3 R1 to R6, §12.2 `_publish_hrrr_run_sync`).

use std::path::Path;

use radar_core::env::Paths;
use radar_core::manifest::packed::{PACKED_KEY, packed_metadata, remove_packed_buckets};
use radar_core::manifest::{FrameSpec, ReplaceOptions, read_manifest, replace_layer};
use radar_tiler::palettes::Palette;
use radar_types::payloads::hrrr::ForecastHourResult;

use super::state::{self, RADAR_HRRR};
use super::{run_init, run_issued_at, valid_time};

/// Publish the complete-layer intersection of the publishable prefix. Returns the layers replaced.
///
/// `palettes` are the loaded HRRR palettes; each layer lists the palettes that carry its color key.
/// `horizon` is the run's horizon (so the caller decides `complete`). Writes only manifest data; tile
/// trees must already be rendered. With `packed` (the paths, when `PACKED_TILES` is on), each layer
/// also lists the packed buckets of its frames' runs, in the same manifest write.
pub fn publish_run(
    state_dir: &Path,
    packed: Option<&Paths>,
    run_id: &str,
    results: &[ForecastHourResult],
    palettes: &[Palette],
    horizon: u32,
) -> anyhow::Result<Vec<String>> {
    let ordered = state::publishable_prefix(results);
    if ordered.is_empty() {
        return Ok(Vec::new());
    }
    let complete = ordered.len() as u32 >= horizon;
    let mut complete_layers: Option<std::collections::BTreeSet<String>> = None;
    for r in &ordered {
        let set: std::collections::BTreeSet<String> = r.rendered_layers.iter().cloned().collect();
        complete_layers = Some(match complete_layers {
            None => set,
            Some(acc) => acc.intersection(&set).cloned().collect(),
        });
    }
    let complete_layers = complete_layers.unwrap_or_default();
    let init = run_init(run_id)?;
    let issued_at = run_issued_at(run_id)?;
    let manifest = read_manifest(state_dir)?;

    let mut published = Vec::new();
    for layer in complete_layers {
        let Some(key) = state::layer_color_key(&layer) else {
            continue;
        };
        let mut palette_names: Vec<String> = palettes
            .iter()
            .filter(|p| p.color_keys().any(|k| k == key))
            .map(|p| p.name().to_string())
            .collect();
        palette_names.sort();
        if palette_names.is_empty() {
            continue;
        }
        let mut frames: Vec<FrameSpec> = Vec::new();
        let mut last_valid = String::new();
        for r in &ordered {
            let valid = valid_time(init, r.fhr as u32);
            let mut extra = serde_json::Map::new();
            extra.insert("source".into(), "hrrr".into());
            extra.insert("kind".into(), "model_guidance".into());
            extra.insert("issued_at".into(), issued_at.clone().into());
            extra.insert("run_id".into(), run_id.into());
            extra.insert("lead_minutes".into(), serde_json::json!(r.fhr * 60));
            extra.insert("spatial_resolution_km".into(), serde_json::json!(3.0));
            extra.insert("max_zoom".into(), serde_json::json!(6));
            last_valid = valid.clone();
            frames.push(FrameSpec {
                timestamp: valid.clone(),
                path: Some(format!("runs/{run_id}/{valid}")),
                palettes: Vec::new(),
                extra,
            });
        }
        let tail = state::extended_tail(manifest.layers.get(&layer), run_id, &last_valid);
        frames.extend(tail);
        let timestamps: Vec<String> = frames.iter().map(|f| f.timestamp.clone()).collect();
        let title = if layer == RADAR_HRRR {
            "HRRR simulated reflectivity".to_string()
        } else {
            format!("HRRR {layer}")
        };
        let mut metadata = serde_json::Map::new();
        metadata.insert("title".into(), title.into());
        metadata.insert("kind".into(), "model_guidance".into());
        metadata.insert("run_id".into(), run_id.into());
        metadata.insert("complete".into(), complete.into());
        match packed.and_then(|paths| packed_value(paths, &layer, &frames)) {
            Some(value) => {
                metadata.insert(PACKED_KEY.into(), value);
            }
            // No buckets: drop stale ones first, so the new frames never map to another run's slots.
            None if manifest
                .layers
                .get(&layer)
                .is_some_and(|l| l.extra.contains_key(PACKED_KEY)) =>
            {
                remove_packed_buckets(state_dir, &layer, |_| true)?;
            }
            None => {}
        }
        let options = ReplaceOptions {
            palettes: palette_names,
            frames,
            layer_metadata: Some(metadata),
        };
        replace_layer(state_dir, &layer, &timestamps, &options)?;
        let _ = init;
        published.push(layer);
    }
    Ok(published)
}

/// The layer's `packed` object for these frames, or `None` (not packed, no buckets, or an error,
/// which is logged: the frames then use their PNG tiles).
fn packed_value(paths: &Paths, layer: &str, frames: &[FrameSpec]) -> Option<serde_json::Value> {
    let layout = super::packed::layout(layer)?;
    let shown: Vec<(String, String)> = frames
        .iter()
        .filter_map(|f| {
            let run = f.extra.get("run_id")?.as_str()?;
            Some((f.timestamp.clone(), run.to_string()))
        })
        .collect();
    match super::packed::run_buckets(paths, layer, &shown) {
        Ok(buckets) => packed_metadata(&layout, buckets),
        Err(err) => {
            tracing::warn!(layer, err = %format!("{err:#}"), "packed_manifest_failed");
            None
        }
    }
}
