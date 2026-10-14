//! JSON round-trip tests for every payload, using example documents from the specs.
//!
//! For each example: decode, re-encode, and check that the encoded document has
//! exactly the example's shape (same keys at every level, same JSON kinds). Then
//! decode the encoded form again and check equality.

use std::collections::BTreeSet;
use std::fmt::Debug;

use radar_types::payloads::{
    airquality::*, alerts::*, cleanup::*, hrrr::*, lightning::*, mrms::*, nowcast::*,
    open_meteo::*, push::*, smoke::*, tropical::*, watch_storm::*,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Decodes `doc`, re-encodes it, and checks shape and value round-trip.
fn check<T>(doc: &str) -> T
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let example: Value = serde_json::from_str(doc).expect("example is valid JSON");
    let parsed: T = serde_json::from_value(example.clone()).expect("example decodes");
    let emitted = serde_json::to_value(&parsed).expect("payload encodes");
    assert_same_shape(&example, &emitted, "$");
    let reparsed: T = serde_json::from_value(emitted).expect("encoded form decodes");
    assert_eq!(parsed, reparsed, "value changed across encode/decode");
    parsed
}

fn assert_same_shape(expected: &Value, actual: &Value, path: &str) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            let ek: BTreeSet<&String> = e.keys().collect();
            let ak: BTreeSet<&String> = a.keys().collect();
            assert_eq!(ek, ak, "key set differs at {path}");
            for (key, ev) in e {
                assert_same_shape(ev, &a[key], &format!("{path}.{key}"));
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            assert_eq!(e.len(), a.len(), "array length differs at {path}");
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                assert_same_shape(ev, av, &format!("{path}[{i}]"));
            }
        }
        (Value::Null, Value::Null)
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_)) => {}
        _ => panic!("JSON kind differs at {path}: expected {expected}, got {actual}"),
    }
}

const SYN_USER: &str = "synthetic-user";
const SYN_ALERT: &str = "synthetic-alert";
const SYN_TOKEN: &str = "synthetic-placeholder";

// ---- mrms -----------------------------------------------------------------

#[test]
fn mrms_args_roundtrip() {
    let v: IngestMrmsArgs =
        check(r#"{"mrms_prefix":"CONUS/MergedBaseReflectivityQC_00.50","layer_name":"radar"}"#);
    assert_eq!(v.layer_name, "radar");
}

#[test]
fn mrms_list_keys_roundtrip() {
    let v: ListKeysResult = check(
        r#"{"keys":["CONUS/MergedBaseReflectivityQC_00.50_20260713-120000.grib2.gz"],"backlog_total":3}"#,
    );
    assert_eq!(v.backlog_total, 3);
}

#[test]
fn mrms_process_frame_input_roundtrip() {
    let _: ProcessFrameInput = check(r#"{"key":"synthetic-key","layer_name":"radar"}"#);
}

#[test]
fn mrms_process_frame_result_roundtrip() {
    let v: ProcessFrameResult = check(
        r#"{"key":"synthetic-key","timestamp":"2026-07-13T12:00:00+00:00","rendered":true,"palettes":["classic","muted","vivid"],"duration_s":41.25}"#,
    );
    assert!(v.rendered);
}

#[test]
fn mrms_mark_processed_input_roundtrip() {
    let _: MarkProcessedInput = check(r#"{"key":"synthetic-key","layer_name":"radar"}"#);
}

#[test]
fn mrms_cleanup_input_and_result_roundtrip() {
    let _: CleanupInput = check(r#"{"layer_name":"radar","retention_hours":4}"#);
    let _: CleanupResult = check(r#"{"tile_dirs_removed":2,"grid_files_removed":5}"#);
}

#[test]
fn mrms_workflow_result_roundtrip() {
    let v: IngestMrmsResult = check(
        r#"{"layer":"radar","backlog_total":3,"rendered_count":1,"cleanup":{"tile_dirs_removed":2,"grid_files_removed":5}}"#,
    );
    assert_eq!(v.cleanup.map(|c| c.grid_files_removed), Some(5));
}

// ---- hrrr -----------------------------------------------------------------

#[test]
fn hrrr_find_run_roundtrip_with_and_without_run() {
    let some: FindRunResult = check(r#"{"run_id":"20260713_12","already_processed":false}"#);
    assert_eq!(some.run_id.as_deref(), Some("20260713_12"));
    let none: FindRunResult = check(r#"{"run_id":null,"already_processed":false}"#);
    assert_eq!(none.run_id, None);
}

#[test]
fn hrrr_forecast_hour_roundtrip() {
    let v: ForecastHourResult = check(
        r#"{"fhr":3,"rendered_layers":["radar-hrrr","temperature"],"duration_s":41.5,"valid_timestamp":"2026-07-13T15:00:00+00:00"}"#,
    );
    assert_eq!(
        v,
        ForecastHourResult {
            fhr: 3,
            rendered_layers: vec!["radar-hrrr".into(), "temperature".into()],
            duration_s: 41.5,
            valid_timestamp: "2026-07-13T15:00:00+00:00".into()
        }
    );
}

#[test]
fn hrrr_cleanup_and_workflow_result_roundtrip() {
    let _: HrrrCleanupResult = check(r#"{"tile_dirs_removed":0,"grid_files_removed":0}"#);
    let v: IngestHrrrResult = check(
        r#"{"run_id":"20260713_12","skipped_already_processed":false,"forecast_hours_processed":2,"cleanup":{"tile_dirs_removed":0,"grid_files_removed":0},"layers_per_hour":[["radar-hrrr"],["radar-hrrr"]],"published_layers":["radar-hrrr"],"complete":false}"#,
    );
    assert_eq!(v.layers_per_hour.len(), 2);
    assert!(!v.complete);
}

// ---- air quality ----------------------------------------------------------

#[test]
fn aqm_payloads_roundtrip() {
    let _: AqmFindRunResult = check(r#"{"run_id":"20260713_06","already_processed":false}"#);
    let _: AqmChunkResult = check(
        r#"{"layer":"air-quality","start_msg":0,"rendered_timestamps":["2026-07-13T06:00:00+00:00"]}"#,
    );
    let _: AqmCleanupResult = check(r#"{"tile_dirs_removed":1}"#);
    let v: IngestAirQualityResult = check(
        r#"{"run_id":"20260713_06","skipped_already_processed":false,"published_layers":["air-quality"],"frames_rendered":24,"cleanup":{"tile_dirs_removed":1}}"#,
    );
    assert_eq!(v.frames_rendered, 24);
}

// ---- lightning, tropical, nowcast, cleanup ---------------------------------

#[test]
fn lightning_result_roundtrip() {
    let v: LightningRunResult = check(
        r#"{"duration_s":3000.1,"msgs":1200,"parsed":1190,"in_bbox":14,"final_buffer":9,"seeded":3,"connect_failures":0}"#,
    );
    assert_eq!(v.msgs, 1200);
}

#[test]
fn tropical_result_roundtrip() {
    let _: TropicalResult = check(r#"{"storm_count":2,"feature_count":7}"#);
}

#[test]
fn nowcast_result_roundtrip_matches_spec_example() {
    let v: NowcastResult = check(
        r#"{"ran":true,"anchor_ts":"2026-07-15T16:00:00+00:00","leadtimes":12,"palettes":["classic","muted","vivid"],"duration_s":512.37,"skill_scored":9}"#,
    );
    assert!(v.ran);
    assert_eq!(v.leadtimes, 12);
}

#[test]
fn tile_cleanup_result_roundtrip() {
    let _: TileCleanupResult =
        check(r#"{"layers_swept":12,"tile_dirs_removed":4,"grid_files_removed":9}"#);
}

// ---- open-meteo -----------------------------------------------------------

#[test]
fn open_meteo_payloads_roundtrip() {
    let args: OpenMeteoSyncArgs = check(
        r#"{"model":"ncep_gfs013","variables":"temperature_2m,relative_humidity_2m","past_days":2}"#,
    );
    assert_eq!(args.past_days, 2);
    let res: OpenMeteoSyncResult = check(
        r#"{"model":"ncep_gfs013","succeeded":true,"duration_s":321.4,"return_code":0,"stderr_tail":""}"#,
    );
    assert!(res.succeeded);
}

// ---- alerts ---------------------------------------------------------------

#[test]
fn alerts_payloads_roundtrip() {
    let alert: AlertForSignal = check(
        r#"{"alert_id":"synthetic-alert","geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}}"#,
    );
    assert_eq!(
        alert.geometry.get("type").and_then(Value::as_str),
        Some("Polygon")
    );
    let _: FetchAlertsResult = check(
        r#"{"alert_count":5,"new_alert_ids":["synthetic-alert"],"new_alerts":[{"alert_id":"synthetic-alert","geometry":{}}]}"#,
    );
    let _: SignalWatchesInput = check(r#"{"alert_id":"synthetic-alert","geometry":{}}"#);
    let _: SignalWatchesResult = check(r#"{"matched":2}"#);
    let _: MarkAlertsSeenInput = check(r#"{"handled_ids":["synthetic-alert"]}"#);
    let v: PollAlertsResult = check(
        r#"{"total":5,"new":1,"signaled_watches":2,"failed_alerts":0,"new_ids":["synthetic-alert"]}"#,
    );
    assert_eq!(v.new_ids, vec![SYN_ALERT.to_owned()]);
}

// ---- watch storm ----------------------------------------------------------

const WATCH_STATE: &str = r#"{"user_id":"synthetic-user","storm_cell_id":"synthetic-cell","lat":42.5,"lng":-85.25,"last_frame_ts":"2026-07-13T12:00:00+00:00","last_max_dbz":42.0,"last_change_kind":"intensifying","last_notified_at":123.0,"poll_count":4001,"push_count":7}"#;

#[test]
fn watch_storm_state_roundtrip() {
    let v: WatchStormState = check(WATCH_STATE);
    assert_eq!(v.push_count, 7);
    assert_eq!(v.last_max_dbz, Some(42.0));
}

#[test]
fn watch_storm_input_without_handoff_roundtrip() {
    let v: WatchStormInput = check(
        r#"{"user_id":"synthetic-user","storm_cell_id":"synthetic-cell","lat":42.5,"lng":-85.25,"handoff":null}"#,
    );
    assert!(v.handoff.is_none());
}

#[test]
fn watch_storm_input_with_handoff_roundtrip() {
    let doc = format!(
        r#"{{"user_id":"synthetic-user","storm_cell_id":"synthetic-cell","lat":42.5,"lng":-85.25,"handoff":{{"schema_version":1,"state":{WATCH_STATE},"pending_alert_ids":["synthetic-alert-a","synthetic-alert-b"]}}}}"#
    );
    let v: WatchStormInput = check(&doc);
    let handoff = v.handoff.expect("handoff present");
    assert_eq!(handoff.schema_version, 1);
    assert_eq!(handoff.pending_alert_ids.len(), 2);
    assert_eq!(handoff.state.last_notified_at, Some(123.0));
}

#[test]
fn watch_storm_handoff_roundtrip() {
    let doc = format!(r#"{{"schema_version":1,"state":{WATCH_STATE},"pending_alert_ids":[]}}"#);
    let v: WatchStormHandoff = check(&doc);
    assert!(v.check_schema_version().is_ok());
}

#[test]
fn compare_and_detect_roundtrip() {
    let _: CompareFramesInput =
        check(r#"{"lat":42.5,"lng":-85.25,"prev_timestamp":null,"prev_max_dbz":null}"#);
    let _: CompareFramesResult = check(
        r#"{"sampled":true,"curr_timestamp":"2026-07-13T12:05:00+00:00","curr_max_dbz":42.0,"prev_max_dbz":35.5,"has_prev":true,"max_dbz_delta":6.5}"#,
    );
    let _: DetectChangeInput =
        check(r#"{"has_prev":true,"curr_max_dbz":42.0,"prev_max_dbz":35.5,"max_dbz_delta":6.5}"#);
    let _: DetectChangeResult = check(r#"{"kind":"intensifying","summary":"Max dBZ rose 6.5"}"#);
}

// ---- push -----------------------------------------------------------------

#[test]
fn push_payloads_roundtrip() {
    let token_doc = format!(r#"{{"user_id":"{SYN_USER}","token":"{SYN_TOKEN}","platform":"ios"}}"#);
    let _: RegisterPushTokenInput = check(&token_doc);
    let _: PushTokenInput = check(&token_doc);
    let _: PushToken = check(&token_doc);
    let _: PushPayload = check(
        r#"{"title":"Storm intensifying","body":"Max dBZ rose 6.5","collapse_id":"watch:abc:2026-07-13T12:05:00+00:00:intensifying","extra":{"storm_cell_id":"synthetic-cell","kind":"intensifying"}}"#,
    );
    let fan: FanOutPushInput = check(
        r#"{"user_id":"synthetic-user","title":"Severe weather alert near your storm","body":"NWS issued an alert in your watched area","collapse_id":"watch:abc:alert:synthetic-alert","extra":{"storm_cell_id":"synthetic-cell","alert_id":"synthetic-alert","kind":"alert"}}"#,
    );
    assert_eq!(fan.extra.get("kind").map(String::as_str), Some("alert"));
    let _: FanOutPushResult = check(r#"{"sent":2}"#);
}

// ---- smoke ----------------------------------------------------------------

#[test]
fn deployment_smoke_roundtrip() {
    let v: DeploymentSmokeResult = check(r#"{"ok":true,"role":"alerts"}"#);
    assert_eq!(v.role, "alerts");
}
