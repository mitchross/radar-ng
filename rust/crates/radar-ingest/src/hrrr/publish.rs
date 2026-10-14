//! Coherent HRRR publication to the manifest (spec §11.3 R1 to R6, §12.2 `_publish_hrrr_run_sync`).

use std::path::Path;

use radar_core::manifest::{FrameSpec, ReplaceOptions, read_manifest, replace_layer};
use radar_tiler::palettes::Palette;
use radar_types::payloads::hrrr::ForecastHourResult;

use super::state::{self, RADAR_HRRR};
use super::{run_init, run_issued_at, valid_time};

/// Publish the complete-layer intersection of the publishable prefix. Returns the layers replaced.
///
/// `palettes` are the loaded HRRR palettes; each layer lists the palettes that carry its color key.
/// `horizon` is the run's horizon (so the caller decides `complete`). Writes only manifest data; tile
/// trees must already be rendered.
pub fn publish_run(
    state_dir: &Path,
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
        let metadata = serde_json::json!({
            "title": title,
            "kind": "model_guidance",
            "run_id": run_id,
            "complete": complete,
        });
        let options = ReplaceOptions {
            palettes: palette_names,
            frames,
            layer_metadata: metadata.as_object().cloned(),
        };
        replace_layer(state_dir, &layer, &timestamps, &options)?;
        let _ = init;
        published.push(layer);
    }
    Ok(published)
}
