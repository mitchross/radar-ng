//! Retry policies and per-activity call options (spec-temporal section 4).
//!
//! These are plain data. The worker crate maps them to SDK options. Every field
//! is written out, including the Python SDK defaults that spec-temporal says a
//! `RetryPolicy(maximum_attempts=N)` inherits (initial 1 s, backoff 2.0, max 100 x initial).
//! Non-retryable errors are signalled by `ApplicationError(non_retryable=True)`, so no
//! error types are listed here.

/// A retry policy. Times are whole seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicySpec {
    /// First retry delay.
    pub initial_interval_s: u64,
    /// Exponential backoff coefficient.
    pub backoff: f64,
    /// Upper bound on the retry delay.
    pub max_interval_s: u64,
    /// Total attempts, including the first.
    pub max_attempts: u32,
}

impl RetryPolicySpec {
    /// Builds a policy. `const` so the tables below can be constants.
    pub const fn new(
        initial_interval_s: u64,
        backoff: f64,
        max_interval_s: u64,
        max_attempts: u32,
    ) -> Self {
        Self {
            initial_interval_s,
            backoff,
            max_interval_s,
            max_attempts,
        }
    }
}

/// Timeouts and retry policy for one activity call. Times are whole seconds.
///
/// `schedule_to_close_s` covers all attempts and queue wait. `heartbeat_s` is the
/// heartbeat timeout. The activity queue is inherited from the calling workflow,
/// except for `open_meteo_sync`, which always uses `OPEN_METEO_TASK_QUEUE`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActivityCallSpec {
    /// Per-attempt timeout.
    pub start_to_close_s: u64,
    /// Overall timeout across attempts and queue wait, if set.
    pub schedule_to_close_s: Option<u64>,
    /// Heartbeat timeout, if the activity heartbeats.
    pub heartbeat_s: Option<u64>,
    /// Retry policy for this call.
    pub retry: RetryPolicySpec,
}

// ---------------------------------------------------------------------------
// Retry policies (spec-ingest-aux section 3.1, spec-ingest-mrms-hrrr section 2.4, spec-temporal section 4)
// ---------------------------------------------------------------------------

/// `_DEFAULT_RETRY`: 1 s, 2.0, 100 s, 5 attempts.
pub const DEFAULT_RETRY: RetryPolicySpec = RetryPolicySpec::new(1, 2.0, 100, 5);
/// MRMS key listing: 1 s, 2.0, 10 s, 3 attempts.
pub const MRMS_LIST_RETRY: RetryPolicySpec = RetryPolicySpec::new(1, 2.0, 10, 3);
/// MRMS frame processing: 2 s, 2.0, 30 s, 2 attempts.
pub const MRMS_FRAME_RETRY: RetryPolicySpec = RetryPolicySpec::new(2, 2.0, 30, 2);
/// HRRR forecast hour: 2 s, 2.0, 120 s, 2 attempts.
pub const HRRR_FORECAST_RETRY: RetryPolicySpec = RetryPolicySpec::new(2, 2.0, 120, 2);
/// AQM chunk render: 5 s, 2.0, 60 s, 2 attempts.
pub const AQM_CHUNK_RETRY: RetryPolicySpec = RetryPolicySpec::new(5, 2.0, 60, 2);
/// Lightning stream: 5 s, 2.0, 120 s, 3 attempts.
pub const LIGHTNING_RETRY: RetryPolicySpec = RetryPolicySpec::new(5, 2.0, 120, 3);
/// Tropical fetch: 2 s, 2.0, 100 s, 5 attempts.
pub const TROPICAL_RETRY: RetryPolicySpec = RetryPolicySpec::new(2, 2.0, 100, 5);
/// Tile cleanup sweep: 2 s, 2.0, 60 s, 3 attempts.
pub const TILE_CLEANUP_RETRY: RetryPolicySpec = RetryPolicySpec::new(2, 2.0, 60, 3);
/// Open-Meteo sync: 10 s, 2.0, 5 min, 3 attempts.
pub const OPEN_METEO_RETRY: RetryPolicySpec = RetryPolicySpec::new(10, 2.0, 300, 3);
/// PollAlerts fetch, signal and mark: 2 s, 2.0, 15 s, 3 attempts.
pub const POLL_ALERTS_RETRY: RetryPolicySpec = RetryPolicySpec::new(2, 2.0, 15, 3);
/// Nowcast: a single attempt (deliberate; see spec-temporal section 4.7).
pub const NOWCAST_RETRY: RetryPolicySpec = RetryPolicySpec::new(1, 2.0, 100, 1);
/// Deployment smoke: 1 s, 2.0, 100 s, 2 attempts.
pub const DEPLOYMENT_SMOKE_RETRY: RetryPolicySpec = RetryPolicySpec::new(1, 2.0, 100, 2);
/// Push token persist and delete: 1 s, 2.0, 10 s, 3 attempts.
pub const PUSH_TOKEN_RETRY: RetryPolicySpec = RetryPolicySpec::new(1, 2.0, 10, 3);
/// WatchStorm `compare_radar_frames` and `detect_storm_change` (`_DEFAULT_RETRY`): 2 s, 2.0, 60 s, 5 attempts.
pub const WATCH_STORM_RETRY: RetryPolicySpec = RetryPolicySpec::new(2, 2.0, 60, 5);
/// WatchStorm push fan-out (`_PUSH_RETRY`): 1 s, 2.0, 30 s, 3 attempts.
pub const WATCH_PUSH_RETRY: RetryPolicySpec = RetryPolicySpec::new(1, 2.0, 30, 3);

// ---------------------------------------------------------------------------
// Activity calls. Names follow the activity they call (`MRMS_PROCESS_FRAME` -> `mrms_process_frame`).
// ---------------------------------------------------------------------------

/// `mrms_list_unprocessed_keys`.
pub const MRMS_LIST_UNPROCESSED_KEYS: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 30,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: MRMS_LIST_RETRY,
};
/// `mrms_process_frame` (at most two in flight per workflow).
pub const MRMS_PROCESS_FRAME: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 240,
    schedule_to_close_s: Some(300),
    heartbeat_s: Some(60),
    retry: MRMS_FRAME_RETRY,
};
/// `mrms_mark_processed`.
pub const MRMS_MARK_PROCESSED: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 30,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};
/// `mrms_cleanup`.
pub const MRMS_CLEANUP: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 300,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};

/// `hrrr_find_latest_run`.
pub const HRRR_FIND_LATEST_RUN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 120,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};
/// `hrrr_horizon_for_run`.
pub const HRRR_HORIZON_FOR_RUN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 10,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};
/// `hrrr_process_forecast_hour` (at most four in flight per workflow).
pub const HRRR_PROCESS_FORECAST_HOUR: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 480,
    schedule_to_close_s: Some(720),
    heartbeat_s: Some(90),
    retry: HRRR_FORECAST_RETRY,
};
/// `hrrr_publish_run`.
pub const HRRR_PUBLISH_RUN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 120,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};
/// `hrrr_mark_processed`.
pub const HRRR_MARK_PROCESSED: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 30,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};
/// `hrrr_cleanup`.
pub const HRRR_CLEANUP: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 300,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};

/// `aqm_find_latest_run`.
pub const AQM_FIND_LATEST_RUN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 120,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};
/// `aqm_render_chunk` (one per layer and start message, at most two in flight).
pub const AQM_RENDER_CHUNK: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 1200,
    schedule_to_close_s: Some(1500),
    heartbeat_s: Some(120),
    retry: AQM_CHUNK_RETRY,
};
/// `aqm_publish_run`.
pub const AQM_PUBLISH_RUN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 120,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};
/// `aqm_mark_processed`.
pub const AQM_MARK_PROCESSED: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 30,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};
/// `aqm_cleanup`.
pub const AQM_CLEANUP: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 300,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: DEFAULT_RETRY,
};

/// `lightning_consume_stream` (argument `3000`).
pub const LIGHTNING_CONSUME_STREAM: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 3060,
    schedule_to_close_s: Some(3300),
    heartbeat_s: Some(90),
    retry: LIGHTNING_RETRY,
};
/// `tropical_fetch_and_publish`. No heartbeat and no schedule-to-close.
pub const TROPICAL_FETCH_AND_PUBLISH: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 60,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: TROPICAL_RETRY,
};
/// `nowcast_run`. One attempt only.
pub const NOWCAST_RUN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 600,
    schedule_to_close_s: Some(720),
    heartbeat_s: Some(120),
    retry: NOWCAST_RETRY,
};
/// `tile_cleanup_sweep`.
pub const TILE_CLEANUP_SWEEP: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 300,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: TILE_CLEANUP_RETRY,
};
/// `open_meteo_sync`. Runs on the open-meteo queue.
pub const OPEN_METEO_SYNC: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 2100,
    schedule_to_close_s: Some(3000),
    heartbeat_s: Some(90),
    retry: OPEN_METEO_RETRY,
};

/// `fetch_nws_active_alerts`.
pub const FETCH_NWS_ACTIVE_ALERTS: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 60,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: POLL_ALERTS_RETRY,
};
/// `signal_matching_storm_watches` (one per new alert with a geometry).
pub const SIGNAL_MATCHING_STORM_WATCHES: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 60,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: POLL_ALERTS_RETRY,
};
/// `mark_alerts_seen`.
pub const MARK_ALERTS_SEEN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 30,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: POLL_ALERTS_RETRY,
};

/// `persist_push_token` (compatibility workflow).
pub const PERSIST_PUSH_TOKEN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 10,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: PUSH_TOKEN_RETRY,
};
/// `delete_push_token` (compatibility workflow).
pub const DELETE_PUSH_TOKEN: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 10,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: PUSH_TOKEN_RETRY,
};

/// `compare_radar_frames` (WatchStorm, one per poll).
pub const COMPARE_RADAR_FRAMES: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 30,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: WATCH_STORM_RETRY,
};
/// `detect_storm_change` (WatchStorm, only on sampled frames).
pub const DETECT_STORM_CHANGE: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 10,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: WATCH_STORM_RETRY,
};
/// `fan_out_push_to_user` (WatchStorm; failures are swallowed by the workflow).
pub const FAN_OUT_PUSH_TO_USER: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 60,
    schedule_to_close_s: None,
    heartbeat_s: None,
    retry: WATCH_PUSH_RETRY,
};

/// `radar_deployment_smoke`. The spec gives `sched=90s, s2c=30s`, read here as
/// schedule-to-close 90 s and start-to-close 30 s.
pub const RADAR_DEPLOYMENT_SMOKE: ActivityCallSpec = ActivityCallSpec {
    start_to_close_s: 30,
    schedule_to_close_s: Some(90),
    heartbeat_s: None,
    retry: DEPLOYMENT_SMOKE_RETRY,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_retry_matches_spec() {
        assert_eq!(DEFAULT_RETRY, RetryPolicySpec::new(1, 2.0, 100, 5));
    }

    #[test]
    fn start_to_close_is_below_schedule_to_close_where_set() {
        for call in [
            MRMS_PROCESS_FRAME,
            HRRR_PROCESS_FORECAST_HOUR,
            AQM_RENDER_CHUNK,
            LIGHTNING_CONSUME_STREAM,
            NOWCAST_RUN,
            OPEN_METEO_SYNC,
            RADAR_DEPLOYMENT_SMOKE,
        ] {
            let sched = call.schedule_to_close_s.expect("schedule-to-close set");
            assert!(call.start_to_close_s < sched, "{call:?}");
        }
    }

    #[test]
    fn spot_check_values_against_spec() {
        assert_eq!(HRRR_PROCESS_FORECAST_HOUR.start_to_close_s, 8 * 60);
        assert_eq!(
            HRRR_PROCESS_FORECAST_HOUR.schedule_to_close_s,
            Some(12 * 60)
        );
        assert_eq!(HRRR_PROCESS_FORECAST_HOUR.heartbeat_s, Some(90));
        assert_eq!(HRRR_PROCESS_FORECAST_HOUR.retry.max_attempts, 2);
        assert_eq!(NOWCAST_RUN.retry.max_attempts, 1);
        assert_eq!(LIGHTNING_CONSUME_STREAM.start_to_close_s, 3060);
        assert_eq!(OPEN_METEO_SYNC.retry, RetryPolicySpec::new(10, 2.0, 300, 3));
        assert_eq!(WATCH_PUSH_RETRY, RetryPolicySpec::new(1, 2.0, 30, 3));
    }
}
