//! `radar`: the single binary for the radar-ng Rust backend.
//!
//! ```text
//! radar api                          # HTTP server (tiles, basemap, /api, /v1) — replaces Caddy + FastAPI + go-pmtiles
//! radar worker --role <role>         # Temporal worker (legacy|all|mrms|nowcast|hrrr|aux|alerts|open-meteo)
//! radar seed                         # create-or-update all Temporal schedules, then exit
//! radar watchdog                     # schedule stall watchdog (normally runs inside the seeding worker)
//! radar debug <check> [...]          # operator diagnostics (port of tools/debug_harness)
//! radar render-once <grib.gz> <out>  # decode + render + storms one MRMS frame and print timings
//! radar parity --python-root A --rust-root B   # decode and compare two tile pyramids
//! ```

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "radar", version, about = "radar-ng backend (Rust)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the HTTP server.
    Api,
    /// Run a Temporal worker for one role.
    Worker {
        /// Worker role; overrides WORKER_ROLE.
        #[arg(long)]
        role: Option<String>,
    },
    /// Create or update every Temporal schedule, print the outcome per schedule, and exit.
    Seed,
    /// Run the schedule watchdog until interrupted.
    Watchdog,
    /// Operator diagnostics.
    Debug(radar_debug::DebugArgs),
    /// Decode, render and detect storms for one MRMS GRIB2 file; print timings.
    RenderOnce { grib: PathBuf, out_dir: PathBuf },
    /// Compare two tile pyramids (Python build vs Rust build) tile by tile, decoded to RGBA.
    ///
    /// The default threshold (--max-diff-pct 10) accepts Python's production legacy RGBA tiles,
    /// which are only ~92% pixel-identical to Rust's indexed tiles by design. Against Python's
    /// indexed renderer the output must be identical: use --exact. See --help for details.
    Parity(radar_debug::parity::ParityArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Offline and CPU-bound (rayon): no runtime, no logging, its own exit status (0 pass, 1 fail,
    // 2 nothing to compare or an error).
    if let Cmd::Parity(args) = &cli.cmd {
        let code = radar_debug::parity::run(args).unwrap_or_else(|e| {
            eprintln!("radar parity: {e:#}");
            2
        });
        std::process::exit(code);
    }

    // Environment overrides must happen before any thread exists (Rust 2024 makes set_var unsafe
    // because it is not thread-safe); the tokio runtime is built afterwards.
    if let Cmd::Worker { role: Some(role) } = &cli.cmd {
        // SAFETY: single-threaded at this point; no other thread reads the environment.
        unsafe { std::env::set_var("WORKER_ROLE", role) };
    }

    let service = match &cli.cmd {
        Cmd::Api => "radar-api",
        Cmd::Worker { .. } | Cmd::Seed | Cmd::Watchdog => "radar-worker",
        Cmd::Debug(_) => "radar-debug",
        Cmd::RenderOnce { .. } => "radar-render-once",
        Cmd::Parity(_) => unreachable!("handled above"),
    };
    // The debug harness prints its own report; keep logs quiet there unless LOG_LEVEL says otherwise.
    if !matches!(cli.cmd, Cmd::Debug(_)) {
        radar_core::logging::init(service).context("initialise logging")?;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("radar")
        .build()
        .context("build tokio runtime")?;

    rt.block_on(async move {
        match cli.cmd {
            Cmd::Api => radar_api::serve(radar_api::Config::from_env()?).await,
            Cmd::Worker { .. } => {
                radar_worker::run_worker(radar_worker::WorkerConfig::from_env()?).await
            }
            Cmd::Seed => {
                let outcomes =
                    radar_worker::seed_schedules(radar_worker::WorkerConfig::from_env()?).await?;
                for (id, outcome) in outcomes {
                    println!("{id}\t{outcome:?}");
                }
                Ok(())
            }
            Cmd::Watchdog => {
                radar_worker::run_watchdog(radar_worker::WorkerConfig::from_env()?).await
            }
            Cmd::Debug(args) => {
                let code = radar_debug::run(args).await?;
                std::process::exit(code);
            }
            Cmd::Parity(_) => unreachable!("handled above"),
            Cmd::RenderOnce { grib, out_dir } => {
                let report = tokio::task::spawn_blocking(move || {
                    radar_ingest::bench_render_once(&grib, &out_dir)
                })
                .await??;
                println!("{report:#?}");
                Ok(())
            }
        }
    })
}
