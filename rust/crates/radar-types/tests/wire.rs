//! Wire names, queue constants, worker roles and queue routing.

use std::collections::HashSet;
use std::str::FromStr;

use radar_types::wire::*;

#[test]
fn workflow_types_are_fourteen_unique_exact_names() {
    assert_eq!(ALL_WORKFLOW_TYPES.len(), 14);
    let unique: HashSet<&str> = ALL_WORKFLOW_TYPES.iter().copied().collect();
    assert_eq!(unique.len(), 14);
    for name in [
        "RadarDeploymentSmokeWorkflow",
        "IngestMrmsWorkflow",
        "IngestHrrrWorkflow",
        "IngestAirQualityWorkflow",
        "IngestLightningWorkflow",
        "IngestTropicalWorkflow",
        "NowcastWorkflow",
        "TileCleanupWorkflow",
        "OpenMeteoSyncWorkflow",
        "PollAlertsWorkflow",
        "WatchStormWorkflow",
        "RegisterPushTokenWorkflow",
        "DeletePushTokenWorkflow",
        "IngestGlobalModelWorkflow",
    ] {
        assert!(ALL_WORKFLOW_TYPES.contains(&name), "missing {name}");
    }
}

#[test]
fn activity_names_are_thirty_five_unique_exact_names() {
    assert_eq!(ALL_ACTIVITY_NAMES.len(), 35);
    let unique: HashSet<&str> = ALL_ACTIVITY_NAMES.iter().copied().collect();
    assert_eq!(unique.len(), 35);
    assert_eq!(
        ACTIVITY_GLOBAL_MODEL_PROCESS_HOUR,
        "global_model_process_hour"
    );
    assert_eq!(ACTIVITY_OPEN_METEO_SYNC, "open_meteo_sync");
    assert_eq!(ACTIVITY_RADAR_DEPLOYMENT_SMOKE, "radar_deployment_smoke");
    assert_eq!(ACTIVITY_SEND_PUSH_NOTIFICATION, "send_push_notification");
    assert_eq!(ACTIVITY_AQM_RENDER_CHUNK, "aqm_render_chunk");
    assert_eq!(
        ACTIVITY_SIGNAL_MATCHING_STORM_WATCHES,
        "signal_matching_storm_watches"
    );
}

#[test]
fn signal_query_and_patch_names_are_exact() {
    assert_eq!(SIGNAL_UNPIN, "unpinSignal");
    assert_eq!(SIGNAL_WAKE_UP, "wakeUpSignal");
    assert_eq!(SIGNAL_ALERT_MATCH, "alertMatchSignal");
    assert_eq!(QUERY_GET_CURRENT_STATE, "getCurrentState");
    assert_eq!(
        PATCH_WATCH_STORM_ALERT_QUEUE_V1,
        "watch-storm-alert-queue-v1"
    );
    assert_eq!(
        PATCH_WATCH_STORM_VERSION_BOUNDARY_V1,
        "watch-storm-version-boundary-v1"
    );
    assert_eq!(PATCH_WATCH_STORM_HANDOFF_V1, "watch-storm-handoff-v1");
}

#[test]
fn task_queue_names_are_exact() {
    assert_eq!(LEGACY_TASK_QUEUE, "radar-ng");
    assert_eq!(MRMS_TASK_QUEUE, "radar-ng-mrms");
    assert_eq!(NOWCAST_TASK_QUEUE, "radar-ng-nowcast");
    assert_eq!(HRRR_TASK_QUEUE, "radar-ng-hrrr");
    assert_eq!(AUX_TASK_QUEUE, "radar-ng-aux");
    assert_eq!(ALERTS_TASK_QUEUE, "radar-ng-alerts");
    assert_eq!(OPEN_METEO_TASK_QUEUE, "radar-ng-open-meteo");
}

#[test]
fn worker_role_parsing_strips_and_lowercases() {
    assert_eq!(WorkerRole::from_str(" MRMS\n").unwrap(), WorkerRole::Mrms);
    assert_eq!(WorkerRole::from_str("Legacy").unwrap(), WorkerRole::Legacy);
    assert_eq!(WorkerRole::from_str("all").unwrap(), WorkerRole::All);
    assert_eq!(
        WorkerRole::from_str("nowcast").unwrap(),
        WorkerRole::Nowcast
    );
    assert_eq!(WorkerRole::from_str("hrrr").unwrap(), WorkerRole::Hrrr);
    assert_eq!(WorkerRole::from_str("aux").unwrap(), WorkerRole::Aux);
    assert_eq!(WorkerRole::from_str("alerts").unwrap(), WorkerRole::Alerts);
    assert_eq!(
        WorkerRole::from_str("open-meteo").unwrap(),
        WorkerRole::OpenMeteo
    );
    assert_eq!(
        WorkerRole::from_str("open_meteo").unwrap(),
        WorkerRole::OpenMeteo
    );
}

#[test]
fn unknown_worker_role_is_an_error_naming_the_input() {
    let err = WorkerRole::from_str("bogus").unwrap_err();
    assert_eq!(err, UnknownWorkerRole("bogus".into()));
    assert!(err.to_string().contains("bogus"));
    assert!(WorkerRole::from_str("").is_err());
}

#[test]
fn role_default_queues_and_round_trip() {
    let expected = [
        (WorkerRole::Legacy, "radar-ng"),
        (WorkerRole::All, "radar-ng"),
        (WorkerRole::Mrms, "radar-ng-mrms"),
        (WorkerRole::Nowcast, "radar-ng-nowcast"),
        (WorkerRole::Hrrr, "radar-ng-hrrr"),
        (WorkerRole::Aux, "radar-ng-aux"),
        (WorkerRole::Alerts, "radar-ng-alerts"),
        (WorkerRole::OpenMeteo, "radar-ng-open-meteo"),
    ];
    assert_eq!(WorkerRole::ALL.len(), expected.len());
    for (role, queue) in expected {
        assert_eq!(role.default_task_queue(), queue);
        assert_eq!(WorkerRole::from_str(role.as_str()).unwrap(), role);
        assert_eq!(role.to_string(), role.as_str());
    }
}

#[test]
fn schedule_task_queue_routing_rule() {
    assert_eq!(schedule_task_queue("radar-ng-mrms", true), "radar-ng-mrms");
    assert_eq!(schedule_task_queue("radar-ng-mrms", false), "radar-ng");
    assert_eq!(
        schedule_task_queue(ALERTS_TASK_QUEUE, false),
        LEGACY_TASK_QUEUE
    );
}

#[test]
fn isolation_flag_is_exact_match() {
    assert!(use_isolated_task_queues(Some("1")));
    assert!(!use_isolated_task_queues(Some("true")));
    assert!(!use_isolated_task_queues(Some("0")));
    assert!(!use_isolated_task_queues(Some(" 1")));
    assert!(!use_isolated_task_queues(None));
}

#[test]
fn poll_queue_uses_override_or_role_default() {
    assert_eq!(poll_task_queue(WorkerRole::Hrrr, None), "radar-ng-hrrr");
    assert_eq!(poll_task_queue(WorkerRole::Hrrr, Some("")), "radar-ng-hrrr");
    assert_eq!(
        poll_task_queue(WorkerRole::Hrrr, Some("custom-queue")),
        "custom-queue"
    );
}

#[test]
fn watch_start_queue_ignores_isolation_and_defaults_to_alerts() {
    assert_eq!(watch_start_task_queue(None), "radar-ng-alerts");
    assert_eq!(watch_start_task_queue(Some("")), "radar-ng-alerts");
    assert_eq!(watch_start_task_queue(Some("alerts-blue")), "alerts-blue");
}

#[test]
fn env_var_names_are_exact() {
    assert_eq!(ENV_WORKER_ROLE, "WORKER_ROLE");
    assert_eq!(ENV_TEMPORAL_TASK_QUEUE, "TEMPORAL_TASK_QUEUE");
    assert_eq!(ENV_USE_ISOLATED_TASK_QUEUES, "USE_ISOLATED_TASK_QUEUES");
    assert_eq!(ENV_SEED_SCHEDULES, "SEED_SCHEDULES");
    assert_eq!(ENV_SKIP_SCHEDULE_SEED, "SKIP_SCHEDULE_SEED");
    assert_eq!(ENV_TEMPORAL_ALERTS_TASK_QUEUE, "TEMPORAL_ALERTS_TASK_QUEUE");
}
