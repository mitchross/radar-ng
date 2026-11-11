//! Synthetic finished revisions for tests; no network, no MRMS data.

use crate::case::{self, CASE_PALETTE, CASE_ZOOMS, CaseDefinition};
use crate::import::{PlannedFrame, frame_dir};
use crate::{objects, plan, revision};
use chrono::{DateTime, Duration, Utc};
use radar_geo::{Grid, GridGeo};
use radar_grib::mrms as gmrms;
use radar_tiler::{FrameKind, FrameOptions, Overview};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn grid(case: &CaseDefinition) -> Grid {
    let [west, south, east, north] = case.bounds;
    let step = 0.1;
    let lats: Vec<f64> = (0..)
        .map(|i| south - 1.0 + step * i as f64)
        .take_while(|v| *v < north + 1.0)
        .collect();
    let lons: Vec<f64> = (0..)
        .map(|i| west - 1.0 + step * i as f64)
        .take_while(|v| *v < east + 1.0)
        .collect();
    let (height, width) = (lats.len(), lons.len());
    Grid {
        height,
        width,
        values: (0..height * width).map(|k| 5.0 + (k % 55) as f32).collect(),
        geo: GridGeo::Regular { lats, lons },
    }
}

fn key_for(spec: &case::LayerSpec, time: DateTime<Utc>) -> String {
    format!(
        "{}/{}/MRMS_{}_{}.grib2.gz",
        spec.prefix,
        time.format("%Y%m%d"),
        spec.product,
        time.format("%Y%m%d-%H%M%S")
    )
}

/// Write a finished whole-case private revision (`public_ready` false) with `per_layer` frames
/// for each layer, the way a full `radar replay import` leaves it. Returns its directory.
pub fn synthetic_revision(
    tile_root: &Path,
    case: &CaseDefinition,
    revision_name: &str,
    per_layer: usize,
) -> anyhow::Result<PathBuf> {
    let layout = revision::layout(tile_root, case, revision_name)?;
    fs::create_dir_all(&layout.staging)?;
    let (from, to) = case.window()?;
    let tiles = plan::case_tiles(&case.bounds, &CASE_ZOOMS);
    let source_grid = grid(case);
    let tables_for =
        |color: &str| radar_ingest::mrms_qpe::load_tables(&[CASE_PALETTE.into()], color);
    let mut coverage_layers = Vec::new();
    let mut listed_by_layer = Vec::new();
    for spec in case.layers() {
        let objects: Vec<(DateTime<Utc>, String)> = (0..per_layer as i64)
            .map(|k| {
                let t = from + Duration::seconds(17 + k * spec.cadence_s);
                (t, key_for(spec, t))
            })
            .collect();
        coverage_layers.push(objects::coverage_for(
            spec.layer,
            spec.prefix,
            spec.cadence_s,
            from,
            to,
            &objects,
        ));
        listed_by_layer.push((spec, objects));
    }
    let base_times: Vec<DateTime<Utc>> = listed_by_layer[0].1.iter().map(|(t, _)| *t).collect();
    let mut frames = Vec::new();
    let mut selection_gaps = BTreeMap::new();
    let mut progress = revision::Progress {
        revision: revision_name.into(),
        case: case.id.into(),
        frames: BTreeMap::new(),
    };
    for (spec, listed) in &listed_by_layer {
        let (_, missing) = plan::choose_frames(&base_times, listed, plan::staleness_for(spec));
        selection_gaps.insert(spec.layer, missing);
        for (time, key) in listed {
            let timestamp = gmrms::format_valid_time(*time);
            let sha = radar_tiler::identity::sha256_hex(format!("synthetic {key}").as_bytes());
            let source_id = format!("replay:{}:{key}:{sha}", spec.layer);
            let output = frame_dir(&layout.staging, spec.layer, &timestamp);
            let options = FrameOptions {
                source_id: source_id.clone(),
                overview: Some(Overview::Max),
                keep_empty: true,
                publication_lock_root: Some(layout.staging.clone()),
                tile_allowlist: Some(tiles.clone()),
                ..FrameOptions::default()
            };
            let kind = FrameKind::Continuous {
                nodata_value: None,
                min_valid_weight: 1.0,
            };
            let result = radar_tiler::render_frame_palettes(
                &source_grid,
                &kind,
                &tables_for(spec.color_key),
                &[(CASE_PALETTE.into(), output.clone())],
                &CASE_ZOOMS,
                &options,
            )?;
            let outcome = result
                .outcome(CASE_PALETTE)
                .ok_or_else(|| anyhow::anyhow!("no outcome"))?;
            frames.push(PlannedFrame {
                layer: spec.layer.into(),
                timestamp: timestamp.clone(),
                key: key.clone(),
                flag_key: None,
            });
            progress.frames.insert(
                format!("{}@{timestamp}", spec.layer),
                revision::FrameProgress {
                    layer: spec.layer.into(),
                    playhead: timestamp.clone(),
                    timestamp,
                    key: key.clone(),
                    source_id,
                    source_sha256: sha,
                    flag_key: None,
                    flag_sha256: None,
                    tiles: outcome.tile_count,
                    bytes: crate::quota::dir_usage(&output)?.bytes,
                    render_ms: 1,
                },
            );
        }
    }
    let coverage = objects::Coverage {
        case: case.id.into(),
        start: case.start.into(),
        end: case.end.into(),
        checked_at: gmrms::format_valid_time(Utc::now()),
        layers: coverage_layers,
        precip_flag: objects::coverage_for(
            "precip-flag",
            case::PRECIP_FLAG_PREFIX,
            120,
            from,
            to,
            &[],
        ),
    };
    revision::write_json_atomic(&layout.staging.join(revision::COVERAGE_FILE), &coverage)?;
    revision::write_json_atomic(&layout.staging.join("selection-gaps.json"), &selection_gaps)?;
    revision::write_json_atomic(&layout.staging.join(revision::PROGRESS_FILE), &progress)?;
    revision::write_json_atomic(
        &layout.staging.join("plan.json"),
        &json!({
            "schema_version": 1, "case": case.id, "revision": revision_name,
            "base_url": "http://synthetic.invalid", "max_frames": null, "layers": [],
            "frames": frames, "tiles": tiles, "verify_crop": false,
        }),
    )?;
    revision::write_json_atomic(
        &layout.staging.join(revision::REPORT_FILE),
        &json!({"synthetic": true}),
    )?;
    revision::write_json_atomic(
        &layout.staging.join("revision.json"),
        &json!({
            "schema_version": 1, "case": case.id, "revision": revision_name,
            "status": "tiles_complete", "public_ready": false,
            "warnings": {"status": "missing", "issue": 282}, "scope": "case",
            "start": case.start, "end": case.end, "bounds": case.bounds, "timezone": case.timezone,
            "sources": [{"name": "NOAA MRMS", "base_url": "http://synthetic.invalid"}],
            "manifest": revision::MANIFEST_FILE, "coverage": revision::COVERAGE_FILE,
            "provenance": revision::PROGRESS_FILE, "plan": "plan.json",
        }),
    )?;
    revision::publish(&layout, case, &progress)
}

/// A valid warnings history for `case`: one tornado warning active at the case start.
pub fn warnings_json(case: &CaseDefinition) -> Vec<u8> {
    warnings_json_with(case, "TORNADO WARNING")
}

/// Like [`warnings_json`], with different bulletin wording (so the file hash differs).
pub fn warnings_json_with(case: &CaseDefinition, wording: &str) -> Vec<u8> {
    let (wfo, lon, lat) = if case.id == "rolling-fork-2023" {
        ("JAN", -91.0, 32.0)
    } else {
        ("DMX", -95.0, 41.0)
    };
    let (start, _) = case.window().expect("case window");
    let begin = start - Duration::minutes(10);
    let finish = start + Duration::minutes(30);
    let iso = |t: DateTime<Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let product = format!("{}-K{wfo}-WFUS54-TOR{wfo}", begin.format("%Y%m%d%H%M"));
    let year: i32 = case.start[..4].parse().expect("year");
    let text = format!(
        "WFUS54 K{wfo} {}\nTOR{wfo}\n\n{wording}\n\n/O.NEW.K{wfo}.TO.W.0001.230324T2250Z-230324T2330Z/\n\n$$\n",
        begin.format("%d%H%M")
    );
    let feature = json!({
        "type": "Feature",
        "id": format!("{year}.{wfo}.TO.W.1/{product}/{}", iso(begin)),
        "geometry": {"type": "Polygon", "coordinates": [[
            [lon, lat], [lon + 1.0, lat], [lon + 1.0, lat + 1.0], [lon, lat + 1.0], [lon, lat]
        ]]},
        "properties": {
            "year": year, "wfo": wfo, "phenomena": "TO", "significance": "W", "eventid": 1,
            "event": "Tornado Warning", "status": "NEW", "product_id": product,
            "utc_polygon_begin": iso(begin), "utc_polygon_end": iso(finish),
            "is_emergency": false, "is_pds": false,
        },
    });
    let doc: Value = json!({
        "type": "FeatureCollection", "schema_version": 1, "case_id": case.id,
        "start": case.start, "end": case.end, "bounds": case.bounds,
        "seed": [feature.clone()], "features": [feature],
        "bulletins": {
            product.clone(): {
                "url": format!("https://mesonet.agron.iastate.edu/api/1/nwstext/{product}?nolimit=true"),
                "sha256": radar_tiler::identity::sha256_hex(text.as_bytes()),
                "text": text,
            }
        },
        "sources": [{
            "url": "https://mesonet.agron.iastate.edu/api/1/vtec/sbw_interval.geojson",
            "sha256": radar_tiler::identity::sha256_hex(b"synthetic response"),
        }],
    });
    serde_json::to_vec(&doc).expect("serialize warnings")
}
