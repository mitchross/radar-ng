//! Process configuration read from the environment (spec-temporal section 3.1).
//!
//! [`WorkerConfig::from_env`] performs every validation before any network or background work starts,
//! so a bad value fails startup the same way the Python `main()` does (section 3.3).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use radar_types::wire::{self, WorkerRole};

/// Default Prometheus bind address for the SDK runtime (`TEMPORAL_METRICS_BIND`).
pub const DEFAULT_METRICS_BIND: &str = "0.0.0.0:9464";
/// Default readiness-style touch file (`TEMPORAL_HEALTH_FILE`).
pub const DEFAULT_HEALTH_FILE: &str = "/tmp/temporal-healthy";
/// Default liveness-style touch file (`TEMPORAL_ALIVE_FILE`).
pub const DEFAULT_ALIVE_FILE: &str = "/tmp/worker-alive";
/// Default activity slot count (`TEMPORAL_MAX_CONCURRENT_ACTIVITIES`).
pub const DEFAULT_MAX_CONCURRENT_ACTIVITIES: i64 = 4;
/// Default activity poller count (`TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS`).
pub const DEFAULT_MAX_ACTIVITY_POLLS: i64 = 2;
/// Default drain budget in seconds (`TEMPORAL_GRACEFUL_SHUTDOWN_S`).
pub const DEFAULT_GRACEFUL_SHUTDOWN_S: i64 = 25;
/// Workflow task slots. The Python worker leaves the SDK default in place; Rust fixes it here.
pub const WORKFLOW_TASK_SLOTS: usize = 100;

/// Worker Versioning identity. Present only when both variables are non-empty (spec 3.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versioning {
    /// `TEMPORAL_DEPLOYMENT_NAME`.
    pub deployment_name: String,
    /// `TEMPORAL_WORKER_BUILD_ID`.
    pub build_id: String,
}

/// Validated worker settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    /// Pool role (`WORKER_ROLE`).
    pub role: WorkerRole,
    /// Poll queue (`TEMPORAL_TASK_QUEUE` or the role default).
    pub task_queue: String,
    /// Whether this process runs the schedule seeder and watchdog (spec 3.3).
    pub seed_schedules: bool,
    /// `USE_ISOLATED_TASK_QUEUES == "1"` (spec 2.2). Read by the seeder.
    pub use_isolated_task_queues: bool,
    /// Worker Versioning identity, or `None` when versioning is off.
    pub versioning: Option<Versioning>,
    /// Activity slots (`TEMPORAL_MAX_CONCURRENT_ACTIVITIES`, min 1).
    pub max_concurrent_activities: usize,
    /// Activity pollers (`TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS`, min 1).
    pub max_concurrent_activity_polls: usize,
    /// Drain budget on shutdown (`TEMPORAL_GRACEFUL_SHUTDOWN_S`, min 1).
    pub graceful_shutdown: Duration,
    /// Prometheus bind address, or `None` when `TEMPORAL_METRICS_BIND=off`.
    pub metrics_bind: Option<SocketAddr>,
    /// Readiness-style touch file (`TEMPORAL_HEALTH_FILE`).
    pub health_file: PathBuf,
    /// Liveness-style touch file (`TEMPORAL_ALIVE_FILE`).
    pub alive_file: PathBuf,
}

impl WorkerConfig {
    /// Read and validate every worker setting from the process environment.
    ///
    /// Fails for an unknown `WORKER_ROLE` or an unparsable metrics bind address. Numeric variables
    /// fall back to their defaults (with a warning) as the Python `int_env` helper does.
    pub fn from_env() -> anyhow::Result<Self> {
        let env = |name: &str| std::env::var(name).ok();
        Self::from_lookup(&env)
    }

    /// Same as [`WorkerConfig::from_env`] with an injectable lookup, for tests.
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let raw_role = get(wire::ENV_WORKER_ROLE).unwrap_or_else(|| "legacy".to_owned());
        let role = WorkerRole::from_str(&raw_role).map_err(|e| anyhow::anyhow!(e))?;

        let task_queue = wire::poll_task_queue(role, get(wire::ENV_TEMPORAL_TASK_QUEUE).as_deref());

        let seed_env = get(wire::ENV_SEED_SCHEDULES);
        let skip_env = get(wire::ENV_SKIP_SCHEDULE_SEED);
        let seed_schedules = radar_types::schedules::should_seed_schedules(
            role,
            seed_env.as_deref(),
            skip_env.as_deref(),
        );

        let use_isolated_task_queues =
            wire::use_isolated_task_queues(get(wire::ENV_USE_ISOLATED_TASK_QUEUES).as_deref());

        let versioning = match (
            get("TEMPORAL_DEPLOYMENT_NAME").filter(|s| !s.is_empty()),
            get("TEMPORAL_WORKER_BUILD_ID").filter(|s| !s.is_empty()),
        ) {
            (Some(deployment_name), Some(build_id)) => Some(Versioning {
                deployment_name,
                build_id,
            }),
            _ => None,
        };

        let max_concurrent_activities = parse_int_setting(
            "TEMPORAL_MAX_CONCURRENT_ACTIVITIES",
            get("TEMPORAL_MAX_CONCURRENT_ACTIVITIES").as_deref(),
            DEFAULT_MAX_CONCURRENT_ACTIVITIES,
            1,
        );
        let max_concurrent_activity_polls = parse_int_setting(
            "TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS",
            get("TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS").as_deref(),
            DEFAULT_MAX_ACTIVITY_POLLS,
            1,
        );
        let graceful_s = parse_int_setting(
            "TEMPORAL_GRACEFUL_SHUTDOWN_S",
            get("TEMPORAL_GRACEFUL_SHUTDOWN_S").as_deref(),
            DEFAULT_GRACEFUL_SHUTDOWN_S,
            1,
        );

        let metrics_bind = parse_metrics_bind(get("TEMPORAL_METRICS_BIND").as_deref())?;

        Ok(Self {
            role,
            task_queue,
            seed_schedules,
            use_isolated_task_queues,
            versioning,
            max_concurrent_activities: max_concurrent_activities as usize,
            max_concurrent_activity_polls: max_concurrent_activity_polls as usize,
            graceful_shutdown: Duration::from_secs(graceful_s as u64),
            metrics_bind,
            health_file: get("TEMPORAL_HEALTH_FILE")
                .filter(|s| !s.is_empty())
                .map_or_else(|| PathBuf::from(DEFAULT_HEALTH_FILE), PathBuf::from),
            alive_file: get("TEMPORAL_ALIVE_FILE")
                .filter(|s| !s.is_empty())
                .map_or_else(|| PathBuf::from(DEFAULT_ALIVE_FILE), PathBuf::from),
        })
    }
}

/// Parse an integer setting the way Python `int_env` does: an absent value returns `default`, an
/// unparsable one logs a warning and returns `default`, and the result is `max(min, value)`.
pub fn parse_int_setting(name: &str, raw: Option<&str>, default: i64, min: i64) -> i64 {
    let value = match raw {
        None => default,
        Some(text) => match text.trim().parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    setting = name,
                    value = text,
                    "invalid integer setting; using default"
                );
                default
            }
        },
    };
    value.max(min)
}

/// Parse `TEMPORAL_METRICS_BIND`. Unset or empty means the default bind; `off` (any case) disables
/// metrics. Any other value must be a socket address.
pub fn parse_metrics_bind(raw: Option<&str>) -> anyhow::Result<Option<SocketAddr>> {
    let text = match raw.map(str::trim) {
        None | Some("") => DEFAULT_METRICS_BIND,
        Some(text) => text,
    };
    if text.eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    let addr: SocketAddr = text
        .parse()
        .with_context(|| format!("TEMPORAL_METRICS_BIND is not a socket address: {text:?}"))?;
    Ok(Some(addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn defaults_for_empty_environment() {
        let cfg = WorkerConfig::from_lookup(&env(&[])).unwrap();
        assert_eq!(cfg.role, WorkerRole::Legacy);
        assert_eq!(cfg.task_queue, wire::LEGACY_TASK_QUEUE);
        assert!(cfg.seed_schedules, "legacy seeds by default");
        assert!(cfg.versioning.is_none());
        assert_eq!(cfg.max_concurrent_activities, 4);
        assert_eq!(cfg.max_concurrent_activity_polls, 2);
        assert_eq!(cfg.graceful_shutdown, Duration::from_secs(25));
        assert_eq!(
            cfg.metrics_bind,
            Some(DEFAULT_METRICS_BIND.parse().unwrap())
        );
        assert_eq!(cfg.health_file, PathBuf::from("/tmp/temporal-healthy"));
        assert_eq!(cfg.alive_file, PathBuf::from("/tmp/worker-alive"));
    }

    #[test]
    fn unknown_role_fails_startup() {
        assert!(WorkerConfig::from_lookup(&env(&[("WORKER_ROLE", "bogus")])).is_err());
    }

    #[test]
    fn role_is_trimmed_and_case_insensitive() {
        let cfg = WorkerConfig::from_lookup(&env(&[("WORKER_ROLE", "  MRMS ")])).unwrap();
        assert_eq!(cfg.role, WorkerRole::Mrms);
        assert_eq!(cfg.task_queue, wire::MRMS_TASK_QUEUE);
        assert!(
            !cfg.seed_schedules,
            "non-legacy roles do not seed by default"
        );
    }

    #[test]
    fn seed_flag_semantics_match_python() {
        // "1" forces seeding on any role.
        assert!(
            WorkerConfig::from_lookup(&env(&[("WORKER_ROLE", "hrrr"), ("SEED_SCHEDULES", "1")]))
                .unwrap()
                .seed_schedules
        );
        // Any other set value disables seeding, even on legacy.
        assert!(
            !WorkerConfig::from_lookup(&env(&[("SEED_SCHEDULES", "0")]))
                .unwrap()
                .seed_schedules
        );
        assert!(
            !WorkerConfig::from_lookup(&env(&[("SEED_SCHEDULES", "true")]))
                .unwrap()
                .seed_schedules
        );
        // SKIP_SCHEDULE_SEED=1 wins.
        assert!(
            !WorkerConfig::from_lookup(&env(&[("SKIP_SCHEDULE_SEED", "1")]))
                .unwrap()
                .seed_schedules
        );
    }

    #[test]
    fn versioning_requires_both_names() {
        let only_name =
            WorkerConfig::from_lookup(&env(&[("TEMPORAL_DEPLOYMENT_NAME", "radar")])).unwrap();
        assert!(only_name.versioning.is_none());
        let empty_build = WorkerConfig::from_lookup(&env(&[
            ("TEMPORAL_DEPLOYMENT_NAME", "radar"),
            ("TEMPORAL_WORKER_BUILD_ID", ""),
        ]))
        .unwrap();
        assert!(empty_build.versioning.is_none());
        let both = WorkerConfig::from_lookup(&env(&[
            ("TEMPORAL_DEPLOYMENT_NAME", "radar"),
            ("TEMPORAL_WORKER_BUILD_ID", "b1"),
        ]))
        .unwrap();
        assert_eq!(
            both.versioning,
            Some(Versioning {
                deployment_name: "radar".into(),
                build_id: "b1".into()
            })
        );
    }

    #[test]
    fn int_settings_fall_back_and_clamp() {
        assert_eq!(parse_int_setting("X", None, 4, 1), 4);
        assert_eq!(parse_int_setting("X", Some("junk"), 4, 1), 4);
        assert_eq!(parse_int_setting("X", Some("0"), 4, 1), 1);
        assert_eq!(parse_int_setting("X", Some(" 9 "), 4, 1), 9);
        let cfg = WorkerConfig::from_lookup(&env(&[
            ("TEMPORAL_MAX_CONCURRENT_ACTIVITIES", "0"),
            ("TEMPORAL_GRACEFUL_SHUTDOWN_S", "-3"),
        ]))
        .unwrap();
        assert_eq!(cfg.max_concurrent_activities, 1);
        assert_eq!(cfg.graceful_shutdown, Duration::from_secs(1));
    }

    #[test]
    fn metrics_bind_off_and_invalid() {
        assert_eq!(parse_metrics_bind(Some("OFF")).unwrap(), None);
        assert_eq!(
            parse_metrics_bind(Some("127.0.0.1:9999")).unwrap(),
            Some("127.0.0.1:9999".parse().unwrap())
        );
        assert!(parse_metrics_bind(Some("not-an-addr")).is_err());
    }

    #[test]
    fn open_meteo_is_accepted_as_its_own_pool() {
        let cfg = WorkerConfig::from_lookup(&env(&[("WORKER_ROLE", "open-meteo")])).unwrap();
        assert_eq!(cfg.role, WorkerRole::OpenMeteo);
        assert_eq!(cfg.task_queue, wire::OPEN_METEO_TASK_QUEUE);
    }
}
