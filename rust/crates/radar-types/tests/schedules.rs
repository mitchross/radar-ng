//! Schedule parity with `temporal/schedules/seed.py` (spec-temporal section 6.2 and section 11, items 1 to 6 and 18).

use radar_types::schedules::{
    GFS013_VARIABLES, GFS025_SURFACE_VARIABLES, HRRR_VARIABLES, OverlapPolicy, ScheduleDef,
    action_workflow_id, all_schedules, schedule_action_queue, should_seed_schedules,
};
use radar_types::wire::WorkerRole;
use serde_json::{Value, json};

/// Expected row: (id, workflow, interval_s, jitter_s, catchup_s, execution_timeout_s, isolated queue, args).
type Row = (
    &'static str,
    &'static str,
    u64,
    Option<u64>,
    u64,
    u64,
    &'static str,
    Value,
);

fn expected_table() -> Vec<Row> {
    vec![
        (
            "ingest-mrms-base",
            "IngestMrmsWorkflow",
            60,
            Some(20),
            300,
            360,
            "radar-ng-mrms",
            json!([{"mrms_prefix": "CONUS/MergedBaseReflectivityQC_00.50", "layer_name": "radar"}]),
        ),
        (
            "ingest-mrms-composite",
            "IngestMrmsWorkflow",
            60,
            Some(20),
            300,
            360,
            "radar-ng-mrms",
            json!([{"mrms_prefix": "CONUS/MergedReflectivityComposite_00.50", "layer_name": "radar-composite"}]),
        ),
        (
            "ingest-hrrr",
            "IngestHrrrWorkflow",
            900,
            None,
            3600,
            2700,
            "radar-ng-hrrr",
            json!([]),
        ),
        (
            "ingest-airquality",
            "IngestAirQualityWorkflow",
            1800,
            None,
            3600,
            4500,
            "radar-ng-aux",
            json!([]),
        ),
        (
            "ingest-lightning",
            "IngestLightningWorkflow",
            3600,
            None,
            3600,
            3360,
            "radar-ng-aux",
            json!([]),
        ),
        (
            "ingest-tropical",
            "IngestTropicalWorkflow",
            3600,
            None,
            3600,
            300,
            "radar-ng-aux",
            json!([]),
        ),
        (
            "nowcast",
            "NowcastWorkflow",
            120,
            Some(20),
            300,
            720,
            "radar-ng-nowcast",
            json!([]),
        ),
        (
            "tile-cleanup",
            "TileCleanupWorkflow",
            3600,
            None,
            3600,
            900,
            "radar-ng-aux",
            json!([]),
        ),
        (
            "poll-alerts",
            "PollAlertsWorkflow",
            300,
            None,
            600,
            270,
            "radar-ng-alerts",
            json!([]),
        ),
        (
            "open-meteo-sync-gfs",
            "OpenMeteoSyncWorkflow",
            21600,
            None,
            3600,
            3600,
            "radar-ng-aux",
            json!([{"model": "ncep_gfs013", "variables": GFS013_VARIABLES, "past_days": 2}]),
        ),
        (
            "open-meteo-sync-gfs025",
            "OpenMeteoSyncWorkflow",
            21600,
            None,
            3600,
            3600,
            "radar-ng-aux",
            json!([{"model": "ncep_gfs025", "variables": GFS025_SURFACE_VARIABLES, "past_days": 2}]),
        ),
        (
            "open-meteo-sync-hrrr",
            "OpenMeteoSyncWorkflow",
            3600,
            None,
            3600,
            3300,
            "radar-ng-aux",
            json!([{"model": "ncep_hrrr_conus", "variables": HRRR_VARIABLES, "past_days": 1}]),
        ),
    ]
}

#[test]
fn all_schedules_matches_spec_table_in_order() {
    let defs = all_schedules();
    let expected = expected_table();
    assert_eq!(defs.len(), 12);
    assert_eq!(defs.len(), expected.len());
    for (def, row) in defs.iter().zip(expected.iter()) {
        let (id, wf, interval, jitter, catchup, timeout, queue, args) = row;
        assert_eq!(def.id, *id);
        assert_eq!(def.workflow_type, *wf, "{id}");
        assert_eq!(def.interval_s, *interval, "{id} interval");
        assert_eq!(def.jitter_s, *jitter, "{id} jitter");
        assert_eq!(def.catchup_window_s, *catchup, "{id} catchup");
        assert_eq!(def.execution_timeout_s, *timeout, "{id} timeout");
        assert_eq!(def.task_queue, *queue, "{id} queue");
        assert_eq!(Value::Array(def.args.clone()), *args, "{id} args");
        assert_eq!(def.overlap, OverlapPolicy::Skip, "{id} overlap");
        assert!(!def.pause_on_failure, "{id} pause_on_failure");
    }
}

#[test]
fn execution_timeout_is_at_most_six_intervals() {
    for def in all_schedules() {
        assert!(def.execution_timeout_s <= 6 * def.interval_s, "{}", def.id);
    }
}

#[test]
fn fast_schedules_are_exactly_three_with_shared_knobs() {
    let defs = all_schedules();
    let fast: Vec<&str> = defs
        .iter()
        .filter(|d| d.interval_s <= 120)
        .map(|d| d.id)
        .collect();
    assert_eq!(
        fast,
        vec!["ingest-mrms-base", "ingest-mrms-composite", "nowcast"]
    );
    for def in defs.iter().filter(|d| d.interval_s <= 120) {
        assert_eq!(def.catchup_window_s, 300, "{}", def.id);
        assert_eq!(def.jitter_s, Some(20), "{}", def.id);
        assert!(def.jitter_s.unwrap() < def.interval_s, "{}", def.id);
    }
}

#[test]
fn slow_schedules_have_one_hour_catchup_and_no_jitter() {
    let defs = all_schedules();
    for id in [
        "ingest-hrrr",
        "ingest-lightning",
        "tile-cleanup",
        "open-meteo-sync-gfs",
    ] {
        let d = defs.iter().find(|d| d.id == id).unwrap();
        assert_eq!(d.catchup_window_s, 3600, "{id}");
        assert_eq!(d.jitter_s, None, "{id}");
    }
    let poll = defs.iter().find(|d| d.id == "poll-alerts").unwrap();
    assert_eq!(poll.catchup_window_s, 600);
}

#[test]
fn open_meteo_variables_exclude_query_time_derived_names() {
    let derived = [
        "weather_code",
        "wind_speed_10m",
        "wind_direction_10m",
        "apparent_temperature",
        "dew_point_2m",
        "surface_pressure",
        "precipitation_probability",
    ];
    for def in all_schedules()
        .iter()
        .filter(|d| d.workflow_type == "OpenMeteoSyncWorkflow")
    {
        let vars = def.args[0]["variables"].as_str().unwrap();
        for name in derived {
            assert!(
                !vars.split(',').any(|v| v == name),
                "{} lists {name}",
                def.id
            );
        }
    }
}

#[test]
fn action_workflow_id_prefix() {
    assert_eq!(action_workflow_id("nowcast"), "sched-nowcast");
    let defs = all_schedules();
    assert_eq!(defs[0].action_workflow_id(), "sched-ingest-mrms-base");
}

#[test]
fn schedule_queue_routing_follows_isolation_flag() {
    let defs = all_schedules();
    let poll: &ScheduleDef = defs.iter().find(|d| d.id == "poll-alerts").unwrap();
    assert_eq!(schedule_action_queue(poll, true), "radar-ng-alerts");
    assert_eq!(schedule_action_queue(poll, false), "radar-ng");
    let hrrr = defs.iter().find(|d| d.id == "ingest-hrrr").unwrap();
    assert_eq!(schedule_action_queue(hrrr, true), "radar-ng-hrrr");
    assert_eq!(schedule_action_queue(hrrr, false), "radar-ng");
}

#[test]
fn seed_gate_matrix_matches_worker_py() {
    use WorkerRole::*;
    // (role, SEED_SCHEDULES, SKIP_SCHEDULE_SEED) -> expected
    let cases: &[(WorkerRole, Option<&str>, Option<&str>, bool)] = &[
        (Legacy, None, None, true),
        (All, None, None, true),
        (Mrms, None, None, false),
        (Aux, None, None, false),
        (Legacy, Some("1"), None, true),
        (Mrms, Some("1"), None, true),
        (Legacy, Some("0"), None, false),
        (Legacy, Some("true"), None, false),
        (All, Some(""), None, false),
        (Mrms, Some("0"), None, false),
        (Legacy, None, Some("1"), false),
        (Mrms, Some("1"), Some("1"), false),
        (Legacy, Some("1"), Some("0"), true),
        (Legacy, Some("1"), Some("true"), true),
        (Legacy, Some(" 1"), None, false),
    ];
    for (role, seed, skip, want) in cases {
        assert_eq!(
            should_seed_schedules(*role, *seed, *skip),
            *want,
            "{role} seed={seed:?} skip={skip:?}"
        );
    }
}
