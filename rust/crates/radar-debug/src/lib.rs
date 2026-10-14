//! radar-debug: the live-stack debug harness, the Rust port of `tools/debug_harness`.
//!
//! It inspects a running radar-ng stack for performance and health problems: data freshness, the
//! API latency budget, tile serving and the `Cache-Control` contract, the app's playback pattern,
//! the on-disk state of the PVCs, and the Temporal control plane. Each check reports `ok`, `warn` or
//! `fail`. Any `fail` makes [`run`] return exit code 1.
//!
//! Entry point: `radar debug <check>` calls [`run`] with the parsed [`DebugArgs`]. The checks that
//! need no server are unit tested in each module. See `README.md` for the command reference.
//!
//! [`parity`] is the offline tile-pyramid comparison behind `radar parity` (Python build against
//! Rust build); it reads two tile roots and needs no running stack.

pub mod checks;
pub mod disk;
pub mod model;
pub mod parity;
pub mod temporal;

use std::time::{Instant, SystemTime};

use checks::{DEFAULT_LAT, DEFAULT_LON};
use clap::{Args, Subcommand};
use disk::DiskRoots;
use model::{Check, Http, exit_code, render_checks, summary_line, utcnow};

/// Version of the harness (`__version__` in the Python).
pub const VERSION: &str = "0.1.0";

/// The `radar debug` arguments: one check to run.
#[derive(Debug, Clone, Args)]
pub struct DebugArgs {
    /// Which check to run.
    #[command(subcommand)]
    pub check: DebugCheck,
}

/// Flags shared by every check (the Python `common` parent parser).
#[derive(Debug, Clone, Args)]
pub struct Common {
    /// Tile-server base URL, the same one the app uses.
    #[arg(
        long,
        env = "RADAR_DEBUG_SERVER",
        default_value = "http://localhost:8080"
    )]
    pub server: String,
    /// Temporal frontend host:port.
    #[arg(
        long = "temporal-address",
        env = "TEMPORAL_ADDRESS",
        default_value = "localhost:7233"
    )]
    pub temporal_address: String,
    /// Temporal namespace.
    #[arg(long, env = "TEMPORAL_NAMESPACE", default_value = "default")]
    pub namespace: String,
    /// Machine-readable output (one JSON document per section).
    #[arg(long)]
    pub json: bool,
    /// Probe latitude for tile and forecast checks.
    #[arg(long, default_value_t = DEFAULT_LAT, allow_hyphen_values = true)]
    pub lat: f64,
    /// Probe longitude for tile and forecast checks.
    #[arg(long, default_value_t = DEFAULT_LON, allow_hyphen_values = true)]
    pub lon: f64,
}

/// The checks, one subcommand each.
#[derive(Debug, Clone, Subcommand)]
pub enum DebugCheck {
    /// Every section in one sweep, with a summary; exits 1 when anything fails.
    Doctor {
        /// Shared flags.
        #[command(flatten)]
        common: Common,
    },
    /// Endpoint latency percentiles against per-endpoint budgets.
    Api {
        /// Shared flags.
        #[command(flatten)]
        common: Common,
        /// Requests per endpoint.
        #[arg(long, default_value_t = 5)]
        samples: usize,
    },
    /// Tile sampling at z4 to z8: latency, 404s, and cache headers.
    Tiles {
        /// Shared flags.
        #[command(flatten)]
        common: Common,
    },
    /// Per-layer data freshness against the schedule cadence.
    Pipeline {
        /// Shared flags.
        #[command(flatten)]
        common: Common,
    },
    /// Replay the app's radar playback fetch pattern against the 420 ms tick.
    Client {
        /// Shared flags.
        #[command(flatten)]
        common: Common,
        /// Viewport zoom level.
        #[arg(long, default_value_t = 7)]
        zoom: u32,
        /// Number of most recent frames to replay.
        #[arg(long, default_value_t = 5)]
        frames: usize,
        /// Layer to replay.
        #[arg(long, default_value = "radar")]
        layer: String,
    },
    /// Schedules, recent failures, stuck workflows, and retrying activities.
    Temporal {
        /// Shared flags.
        #[command(flatten)]
        common: Common,
        /// How far back to look for failed runs, hours.
        #[arg(long, default_value_t = 6)]
        failure_window_h: i64,
    },
    /// PVC headroom, orphaned staging dirs, and manifest-vs-disk drift (run inside a pod).
    Disk {
        /// Shared flags.
        #[command(flatten)]
        common: Common,
    },
    /// Run `doctor` on a loop, for watching a deploy or an incident.
    Watch {
        /// Shared flags.
        #[command(flatten)]
        common: Common,
        /// Seconds between sweeps.
        #[arg(long, default_value_t = 60)]
        interval: u64,
        /// Skip the Temporal section.
        #[arg(long)]
        skip_temporal: bool,
    },
}

/// Run the selected check and return the process exit code (0 healthy, 1 any failure).
pub async fn run(args: DebugArgs) -> anyhow::Result<i32> {
    let http = Http::new()?;
    match args.check {
        DebugCheck::Doctor { common } => {
            let checks = doctor(&http, &common, false).await;
            if !common.json {
                println!("{}", summary_line(&checks));
            }
            Ok(exit_code(&checks))
        }
        DebugCheck::Api { common, samples } => {
            let checks = checks::check_api(&http, &common.server, samples).await;
            render_checks("api — endpoint latency", &checks, common.json, utcnow());
            Ok(exit_code(&checks))
        }
        DebugCheck::Tiles { common } => {
            let checks =
                checks::check_tiles(&http, &common.server, common.lat, common.lon, 9).await;
            render_checks("tiles — serving path", &checks, common.json, utcnow());
            Ok(exit_code(&checks))
        }
        DebugCheck::Pipeline { common } => {
            let checks = checks::check_pipeline(&http, &common.server).await;
            render_checks("pipeline — data freshness", &checks, common.json, utcnow());
            Ok(exit_code(&checks))
        }
        DebugCheck::Client {
            common,
            zoom,
            frames,
            layer,
        } => {
            let checks = checks::check_client_sim(
                &http,
                &common.server,
                common.lat,
                common.lon,
                zoom,
                frames,
                &layer,
            )
            .await;
            render_checks(
                "client — playback simulation",
                &checks,
                common.json,
                utcnow(),
            );
            Ok(exit_code(&checks))
        }
        DebugCheck::Temporal {
            common,
            failure_window_h,
        } => {
            let checks = temporal::check_temporal(
                &common.temporal_address,
                &common.namespace,
                failure_window_h,
                2,
                utcnow(),
            )
            .await;
            render_checks("temporal — control plane", &checks, common.json, utcnow());
            Ok(exit_code(&checks))
        }
        DebugCheck::Disk { common } => {
            let checks = disk::check_disk(&DiskRoots::from_env(), SystemTime::now());
            render_checks("disk — volumes and state", &checks, common.json, utcnow());
            Ok(exit_code(&checks))
        }
        DebugCheck::Watch {
            common,
            interval,
            skip_temporal,
        } => loop {
            let started = Instant::now();
            println!(
                "\n──── sweep @ {} ────",
                chrono::Local::now().format("%H:%M:%S")
            );
            doctor(&http, &common, skip_temporal).await;
            let pause = std::time::Duration::from_secs(interval).saturating_sub(started.elapsed());
            tokio::select! {
                _ = tokio::time::sleep(pause) => {}
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\nstopped");
                    return Ok(0);
                }
            }
        },
    }
}

/// Run every section, render each as it is ready, and return all checks.
async fn doctor(http: &Http, common: &Common, skip_temporal: bool) -> Vec<Check> {
    let mut sections: Vec<(&str, Vec<Check>)> = vec![
        (
            "pipeline — data freshness",
            checks::check_pipeline(http, &common.server).await,
        ),
        (
            "api — endpoint latency",
            checks::check_api(http, &common.server, 5).await,
        ),
        (
            "tiles — serving path",
            checks::check_tiles(http, &common.server, common.lat, common.lon, 9).await,
        ),
        (
            "client — playback simulation",
            checks::check_client_sim(http, &common.server, common.lat, common.lon, 7, 5, "radar")
                .await,
        ),
    ];
    let roots = DiskRoots::from_env();
    if roots.any_present() {
        sections.push((
            "disk — volumes and state",
            disk::check_disk(&roots, SystemTime::now()),
        ));
    }
    if !skip_temporal {
        sections.push((
            "temporal — control plane",
            temporal::check_temporal(&common.temporal_address, &common.namespace, 6, 2, utcnow())
                .await,
        ));
    }
    let mut all = Vec::new();
    for (title, checks) in sections {
        render_checks(title, &checks, common.json, utcnow());
        all.extend(checks);
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, Parser)]
    struct Wrap {
        #[command(flatten)]
        args: DebugArgs,
    }

    fn parse(argv: &[&str]) -> DebugCheck {
        let mut full = vec!["radar-debug"];
        full.extend_from_slice(argv);
        Wrap::try_parse_from(full).expect("parse").args.check
    }

    #[test]
    fn command_definition_is_valid() {
        Wrap::command().debug_assert();
    }

    #[test]
    fn doctor_defaults_match_python() {
        match parse(&["doctor"]) {
            DebugCheck::Doctor { common } => {
                assert_eq!(common.server, "http://localhost:8080");
                assert_eq!(common.temporal_address, "localhost:7233");
                assert_eq!(common.namespace, "default");
                assert!(!common.json);
                assert_eq!(common.lat, 39.5);
                assert_eq!(common.lon, -98.35);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn shared_flags_and_negative_longitude_parse() {
        match parse(&[
            "tiles",
            "--server",
            "http://h:9",
            "--json",
            "--lat",
            "30",
            "--lon",
            "-90.5",
        ]) {
            DebugCheck::Tiles { common } => {
                assert_eq!(common.server, "http://h:9");
                assert!(common.json);
                assert_eq!(common.lat, 30.0);
                assert_eq!(common.lon, -90.5);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn per_check_flags() {
        match parse(&["api", "--samples", "3"]) {
            DebugCheck::Api { samples, .. } => assert_eq!(samples, 3),
            other => panic!("unexpected {other:?}"),
        }
        match parse(&[
            "client", "--zoom", "6", "--frames", "10", "--layer", "nowcast",
        ]) {
            DebugCheck::Client {
                zoom,
                frames,
                layer,
                ..
            } => {
                assert_eq!((zoom, frames, layer.as_str()), (6, 10, "nowcast"));
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse(&["temporal", "--failure-window-h", "12"]) {
            DebugCheck::Temporal {
                failure_window_h, ..
            } => assert_eq!(failure_window_h, 12),
            other => panic!("unexpected {other:?}"),
        }
        match parse(&["watch", "--interval", "5", "--skip-temporal"]) {
            DebugCheck::Watch {
                interval,
                skip_temporal,
                ..
            } => {
                assert_eq!(interval, 5);
                assert!(skip_temporal);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn every_check_name_is_accepted() {
        for name in [
            "doctor", "api", "tiles", "pipeline", "client", "temporal", "disk", "watch",
        ] {
            parse(&[name]);
        }
    }

    #[test]
    fn unknown_check_is_rejected() {
        assert!(Wrap::try_parse_from(["radar-debug", "nope"]).is_err());
    }
}
