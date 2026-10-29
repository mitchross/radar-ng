//! Pure HRRR rules: resume of an incomplete run, the publishable prefix, the extended tail, and the
//! variables and palette keys each layer needs.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};
use radar_core::manifest::FrameSpec;
use radar_core::manifest::{Layer, Manifest};
use radar_core::pycompat::is_falsy;
use radar_grib::hrrr::{HrrrVar, parse_run_id};
use radar_types::payloads::hrrr::ForecastHourResult;

/// Layer whose frames decide resume and coherence.
pub const RADAR_HRRR: &str = "radar-hrrr";

/// Resume an incomplete run younger than 3 h (spec §12.2 `_resume_incomplete_run`). Returns the
/// `radar-hrrr` run id when the layer is incomplete (`complete` present and falsy), its run id parses,
/// and `now - init <= 3 h`. An absent `complete` key means complete.
pub fn resume_incomplete_run(manifest: &Manifest, now: DateTime<Utc>) -> Option<String> {
    let layer = manifest.layers.get(RADAR_HRRR)?;
    let run = layer.extra.get("run_id")?.as_str()?;
    match layer.extra.get("complete") {
        None => return None,
        Some(v) if !is_falsy(v) => return None,
        Some(_) => {}
    }
    let init = parse_run_id(run)?;
    if now - init <= ChronoDuration::hours(3) {
        Some(run.to_string())
    } else {
        None
    }
}

/// `publishable_prefix`: results sorted by fhr, starting at 1, while each hour is present and rendered
/// `radar-hrrr`. The first gap ends the prefix.
pub fn publishable_prefix(results: &[ForecastHourResult]) -> Vec<&ForecastHourResult> {
    let mut sorted: Vec<&ForecastHourResult> = results.iter().collect();
    sorted.sort_by_key(|r| r.fhr);
    // The k-th result (0-based) must be hour k + 1 and rendered. The first failure ends the prefix.
    sorted
        .into_iter()
        .enumerate()
        .take_while(|(index, r)| {
            let expected = i64::try_from(*index).unwrap_or(i64::MAX).saturating_add(1);
            r.fhr == expected && r.rendered_layers.iter().any(|l| l == RADAR_HRRR)
        })
        .map(|(_, r)| r)
        .collect()
}

/// Frames of an older extended run that stay valid after this run's last valid time (spec §11.3 R5,
/// `_extended_tail`). Keeps frames whose run is an earlier extended (00/06/12/18z) run no more than 12 h
/// before this run, and whose valid time is strictly after `last_valid`.
pub fn extended_tail(layer: Option<&Layer>, run_id: &str, last_valid: &str) -> Vec<FrameSpec> {
    let Some(layer) = layer else {
        return Vec::new();
    };
    let Some(run_init) = parse_run_id(run_id) else {
        return Vec::new();
    };
    let Some(last) = radar_core::time::parse_iso_as_utc(last_valid) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for frame in &layer.frames {
        let Some(frame_run) = frame.extra.get("run_id").and_then(|v| v.as_str()) else {
            continue;
        };
        if frame_run == run_id {
            continue;
        }
        let Some(frame_init) = parse_run_id(frame_run) else {
            continue;
        };
        if ![0, 6, 12, 18].contains(&frame_init.hour()) {
            continue;
        }
        if frame_init >= run_init || run_init - frame_init > ChronoDuration::hours(12) {
            continue;
        }
        let Some(valid) = radar_core::time::parse_iso_as_utc(&frame.timestamp) else {
            continue;
        };
        if valid <= last {
            continue;
        }
        out.push(FrameSpec {
            timestamp: frame.timestamp.clone(),
            path: Some(frame.path.clone()),
            palettes: frame.palettes.clone(),
            extra: frame.extra.clone(),
        });
    }
    out
}

/// Runs referenced by a layer: its own `run_id` plus each frame's `run_id` (spec §12.2 `_referenced_runs`).
pub fn referenced_runs(layer: Option<&Layer>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(layer) = layer else { return out };
    if let Some(run) = layer.extra.get("run_id").and_then(|v| v.as_str()) {
        out.insert(run.to_string());
    }
    for frame in &layer.frames {
        if let Some(run) = frame.extra.get("run_id").and_then(|v| v.as_str()) {
            out.insert(run.to_string());
        }
    }
    out
}

/// Variables needed by the enabled layers, sorted unique by key (spec §12.2 `_needed_vars`).
pub fn needed_vars(enabled: &[String]) -> Vec<HrrrVar> {
    let mut vars: Vec<HrrrVar> = Vec::new();
    for layer in enabled {
        vars.extend(layer_vars(layer));
    }
    vars.sort_by_key(|v| v.key());
    vars.dedup();
    vars
}

/// The variables a layer reads (spec §12.2 `LAYER_VARS`).
pub fn layer_vars(layer: &str) -> Vec<HrrrVar> {
    match layer {
        "radar-hrrr" => vec![HrrrVar::Refc],
        "temperature" => vec![HrrrVar::T2m],
        "feels-like" => vec![HrrrVar::T2m, HrrrVar::Rh2m, HrrrVar::U10, HrrrVar::V10],
        "dewpoint" => vec![HrrrVar::Dpt2m],
        "humidity" => vec![HrrrVar::Rh2m],
        "wind" => vec![HrrrVar::U10, HrrrVar::V10],
        "cape" => vec![HrrrVar::Cape],
        "precip-type" => vec![
            HrrrVar::Crain,
            HrrrVar::Csnow,
            HrrrVar::Cfrzr,
            HrrrVar::Cicep,
        ],
        "precip-accum" => vec![HrrrVar::Apcp],
        "cloud" => vec![HrrrVar::Tcdc],
        _ => Vec::new(),
    }
}

/// Palette color key of a layer (spec §6.1 `LAYER_COLOR_KEYS`).
pub fn layer_color_key(layer: &str) -> Option<&'static str> {
    Some(match layer {
        "radar-hrrr" => "reflectivity",
        "temperature" | "feels-like" => "temperature",
        "dewpoint" => "dewpoint",
        "humidity" => "humidity",
        "wind" => "wind_speed",
        "cape" => "cape",
        "precip-type" => "precip_type",
        "precip-accum" => "precip_accum",
        "cloud" => "cloud_cover",
        _ => return None,
    })
}
