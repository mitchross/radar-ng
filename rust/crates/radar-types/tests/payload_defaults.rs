//! Decoding rules: missing defaulted keys, unknown-key tolerance, required keys,
//! JSON integers for floats, and rejection of non-finite floats on encode.

use radar_types::payloads::airquality::{AqmChunkResult, IngestAirQualityResult};
use radar_types::payloads::alerts::{
    AlertForSignal, FetchAlertsResult, MarkAlertsSeenInput, PollAlertsResult,
};
use radar_types::payloads::cleanup::TileCleanupResult;
use radar_types::payloads::hrrr::{ForecastHourResult, IngestHrrrResult};
use radar_types::payloads::lightning::LightningRunResult;
use radar_types::payloads::mrms::{
    DEFAULT_LAYER_NAME, DEFAULT_MRMS_PREFIX, IngestMrmsArgs, IngestMrmsResult, ListKeysResult,
    ProcessFrameInput, ProcessFrameResult,
};
use radar_types::payloads::nowcast::NowcastResult;
use radar_types::payloads::open_meteo::{OpenMeteoSyncArgs, OpenMeteoSyncResult};
use radar_types::payloads::push::FanOutPushInput;
use radar_types::payloads::watch_storm::{
    CompareFramesResult, DetectChangeInput, UnsupportedHandoffSchema,
    WATCH_STORM_HANDOFF_SCHEMA_VERSION, WatchStormHandoff, WatchStormInput, WatchStormState,
};
use serde_json::json;

#[test]
fn missing_defaulted_fields_take_python_defaults() {
    let pf: ProcessFrameResult = serde_json::from_str(r#"{"key":"synthetic-key"}"#).unwrap();
    assert_eq!(pf.timestamp, "");
    assert!(!pf.rendered);
    assert!(pf.palettes.is_empty());
    assert_eq!(pf.duration_s, 0.0);

    let keys: ListKeysResult = serde_json::from_str("{}").unwrap();
    assert!(keys.keys.is_empty());
    assert_eq!(keys.backlog_total, 0);

    let mrms: IngestMrmsResult =
        serde_json::from_str(r#"{"layer":"radar","backlog_total":0,"rendered_count":0}"#).unwrap();
    assert_eq!(mrms.cleanup, None);

    let hrrr: IngestHrrrResult = serde_json::from_str(
        r#"{"run_id":null,"skipped_already_processed":false,"forecast_hours_processed":0}"#,
    )
    .unwrap();
    assert_eq!(hrrr.cleanup, None);
    assert!(hrrr.layers_per_hour.is_empty());
    assert!(hrrr.published_layers.is_empty());
    assert!(
        !hrrr.complete,
        "IngestHrrrResult.complete defaults to false"
    );

    let aqm: IngestAirQualityResult =
        serde_json::from_str(r#"{"run_id":null,"skipped_already_processed":false}"#).unwrap();
    assert!(aqm.published_layers.is_empty());
    assert_eq!(aqm.frames_rendered, 0);
    assert_eq!(aqm.cleanup, None);

    let chunk: AqmChunkResult =
        serde_json::from_str(r#"{"layer":"ozone","start_msg":24}"#).unwrap();
    assert!(chunk.rendered_timestamps.is_empty());

    let light: LightningRunResult = serde_json::from_str(
        r#"{"duration_s":1.0,"msgs":0,"parsed":0,"in_bbox":0,"final_buffer":0}"#,
    )
    .unwrap();
    assert_eq!((light.seeded, light.connect_failures), (0, 0));

    let fh: ForecastHourResult = serde_json::from_str(r#"{"fhr":1}"#).unwrap();
    assert_eq!(fh, ForecastHourResult::failed(1));

    let nc: NowcastResult = serde_json::from_str(r#"{"ran":false}"#).unwrap();
    assert_eq!(nc.anchor_ts, None);
    assert_eq!((nc.leadtimes, nc.skill_scored), (0, 0));
    assert!(nc.palettes.is_empty());
    assert_eq!(nc.duration_s, 0.0);

    let cleanup: TileCleanupResult =
        serde_json::from_str(r#"{"layers_swept":0,"tile_dirs_removed":0,"grid_files_removed":0}"#)
            .unwrap();
    assert_eq!(cleanup.layers_swept, 0);
}

#[test]
fn open_meteo_defaults() {
    let args: OpenMeteoSyncArgs =
        serde_json::from_str(r#"{"model":"ncep_hrrr_conus","variables":"cape"}"#).unwrap();
    assert_eq!(
        args.past_days, 1,
        "OpenMeteoSyncArgs.past_days defaults to 1"
    );

    let res: OpenMeteoSyncResult =
        serde_json::from_str(r#"{"model":"ncep_hrrr_conus","succeeded":false,"duration_s":3.0}"#)
            .unwrap();
    assert_eq!(res.return_code, 0);
    assert_eq!(res.stderr_tail, "");
}

#[test]
fn alert_geometry_null_or_absent_becomes_empty_object() {
    let absent: AlertForSignal = serde_json::from_str(r#"{"alert_id":"synthetic-alert"}"#).unwrap();
    assert!(absent.geometry.is_empty());
    let null: AlertForSignal =
        serde_json::from_str(r#"{"alert_id":"synthetic-alert","geometry":null}"#).unwrap();
    assert!(null.geometry.is_empty());

    let fetched: FetchAlertsResult = serde_json::from_str(r#"{"alert_count":0}"#).unwrap();
    assert!(fetched.new_alert_ids.is_empty() && fetched.new_alerts.is_empty());

    let seen: MarkAlertsSeenInput = serde_json::from_str("{}").unwrap();
    assert!(seen.handled_ids.is_empty());

    let poll: PollAlertsResult = serde_json::from_str(r#"{"total":1,"new":0}"#).unwrap();
    assert_eq!((poll.signaled_watches, poll.failed_alerts), (0, 0));
    assert!(poll.new_ids.is_empty());
}

#[test]
fn watch_storm_state_defaults_and_empty_object() {
    let state: WatchStormState = serde_json::from_str("{}").unwrap();
    assert_eq!(state, WatchStormState::default());
    assert_eq!(state.user_id, "");
    assert_eq!(state.lat, 0.0);
    assert_eq!(state.last_frame_ts, None);
    assert_eq!(state.poll_count, 0);
}

#[test]
fn watch_storm_handoff_defaults_schema_version_to_one() {
    let h: WatchStormHandoff = serde_json::from_str("{}").unwrap();
    assert_eq!(h.schema_version, WATCH_STORM_HANDOFF_SCHEMA_VERSION);
    assert_eq!(h.state, WatchStormState::default());
    assert!(h.pending_alert_ids.is_empty());
    assert!(h.check_schema_version().is_ok());
}

#[test]
fn watch_storm_input_without_handoff_decodes_as_none() {
    let input: WatchStormInput = serde_json::from_str(
        r#"{"user_id":"synthetic-user","storm_cell_id":"synthetic-cell","lat":42.5,"lng":-85.25}"#,
    )
    .unwrap();
    assert!(input.handoff.is_none());
}

#[test]
fn unsupported_handoff_schema_is_rejected_by_check() {
    let h: WatchStormHandoff = serde_json::from_str(r#"{"schema_version":2}"#).unwrap();
    let err = h.check_schema_version().unwrap_err();
    assert_eq!(err, UnsupportedHandoffSchema { found: 2 });
    assert_eq!(err.to_string(), "Unsupported storm-watch handoff schema");

    let zero: WatchStormHandoff = serde_json::from_str(r#"{"schema_version":0}"#).unwrap();
    assert!(zero.check_schema_version().is_err());
}

#[test]
fn fan_out_extra_defaults_to_empty() {
    let fan: FanOutPushInput = serde_json::from_str(
        r#"{"user_id":"synthetic-user","title":"t","body":"b","collapse_id":"c"}"#,
    )
    .unwrap();
    assert!(fan.extra.is_empty());
}

#[test]
fn mrms_args_missing_keys_use_defaults() {
    // The Python defaults come from env MRMS_PREFIX / LAYER_NAME. These tests
    // only assert the built-in defaults, and skip the check if the env sets them.
    let args: IngestMrmsArgs = serde_json::from_str("{}").unwrap();
    if std::env::var_os("MRMS_PREFIX").is_none() {
        assert_eq!(args.mrms_prefix, DEFAULT_MRMS_PREFIX);
    }
    if std::env::var_os("LAYER_NAME").is_none() {
        assert_eq!(args.layer_name, DEFAULT_LAYER_NAME);
    }
    if std::env::var_os("MRMS_PREFIX").is_none() && std::env::var_os("LAYER_NAME").is_none() {
        assert_eq!(IngestMrmsArgs::default(), args);
    }
}

#[test]
fn unknown_keys_are_ignored() {
    let pf: ProcessFrameInput = serde_json::from_value(json!({
        "key": "synthetic-key",
        "layer_name": "radar",
        "future_field": {"nested": [1, 2, 3]},
        "another": null
    }))
    .unwrap();
    assert_eq!(pf.key, "synthetic-key");

    let state: WatchStormState = serde_json::from_value(json!({
        "user_id": "synthetic-user",
        "added_in_v2": true
    }))
    .unwrap();
    assert_eq!(state.user_id, "synthetic-user");

    let poll: PollAlertsResult = serde_json::from_value(json!({
        "total": 1, "new": 1, "unknown_counter": 9
    }))
    .unwrap();
    assert_eq!(poll.new, 1);
}

#[test]
fn required_fields_still_required() {
    assert!(serde_json::from_str::<ProcessFrameInput>(r#"{"key":"synthetic-key"}"#).is_err());
    assert!(serde_json::from_str::<OpenMeteoSyncArgs>(r#"{"model":"ncep_gfs013"}"#).is_err());
    assert!(serde_json::from_str::<PollAlertsResult>(r#"{"new":0}"#).is_err());
    assert!(serde_json::from_str::<DetectChangeInput>(r#"{"has_prev":true}"#).is_err());
}

#[test]
fn float_fields_accept_json_integers() {
    let cmp: CompareFramesResult = serde_json::from_value(json!({
        "sampled": true,
        "curr_timestamp": "2026-07-13T12:05:00+00:00",
        "curr_max_dbz": 42,
        "prev_max_dbz": 35,
        "has_prev": true,
        "max_dbz_delta": 7
    }))
    .unwrap();
    assert_eq!(cmp.curr_max_dbz, 42.0);
    assert_eq!(cmp.max_dbz_delta, 7.0);

    let state: WatchStormState = serde_json::from_value(json!({
        "last_max_dbz": 42,
        "last_notified_at": 123,
        "lat": 1,
        "lng": 0
    }))
    .unwrap();
    assert_eq!(state.last_max_dbz, Some(42.0));
    assert_eq!(state.last_notified_at, Some(123.0));
}

#[test]
fn encoder_emits_every_field_including_nulls() {
    let encoded = serde_json::to_value(WatchStormState::default()).unwrap();
    let obj = encoded.as_object().unwrap();
    assert_eq!(obj.len(), 10);
    assert_eq!(obj["last_frame_ts"], json!(null));
    assert_eq!(obj["lat"], json!(0.0));
}

#[test]
fn non_finite_floats_fail_to_encode() {
    let fh = ForecastHourResult {
        fhr: 1,
        duration_s: f64::NAN,
        ..Default::default()
    };
    assert!(serde_json::to_string(&fh).is_err());

    let state = WatchStormState {
        last_max_dbz: Some(f64::NEG_INFINITY),
        ..Default::default()
    };
    assert!(serde_json::to_string(&state).is_err());

    let detect = DetectChangeInput {
        has_prev: false,
        curr_max_dbz: f64::INFINITY,
        prev_max_dbz: 0.0,
        max_dbz_delta: 0.0,
    };
    assert!(serde_json::to_string(&detect).is_err());

    let finite = WatchStormState {
        last_max_dbz: Some(-99.0),
        ..Default::default()
    };
    assert!(serde_json::to_string(&finite).is_ok());
}

#[test]
fn watch_storm_input_round_trips_to_wire_keys() {
    let input = WatchStormInput {
        user_id: "synthetic-user".into(),
        storm_cell_id: "synthetic-cell".into(),
        lat: 42.5,
        lng: -85.25,
        handoff: None,
    };
    let v = serde_json::to_value(&input).unwrap();
    assert_eq!(
        v,
        json!({
            "user_id": "synthetic-user",
            "storm_cell_id": "synthetic-cell",
            "lat": 42.5,
            "lng": -85.25,
            "handoff": null
        })
    );
}
