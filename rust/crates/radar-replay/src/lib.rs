//! Bounded famous-storm replay importer (design `docs/develop/design/famous-storm-replay.md`).
//!
//! Operator-only tile groundwork; revisions remain private until warnings and API acceptance.

pub mod case;
pub mod import;
pub mod objects;
pub mod plan;
pub mod publish;
pub mod quota;
pub mod revision;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

use clap::{Parser, Subcommand};

/// `radar replay ...` command tree.
#[derive(Debug, Parser)]
pub struct ReplayArgs {
    #[command(subcommand)]
    pub cmd: ReplayCmd,
}

/// Replay subcommands.
#[derive(Debug, Subcommand)]
pub enum ReplayCmd {
    /// Check a case's archive coverage and report gaps; writes nothing.
    Check(CheckArgs),
    /// Import one case into an isolated, revisioned tile root.
    Import(ImportArgs),
    /// Publish a finished whole-case revision as a new public revision with its warnings.
    Publish(PublishArgs),
}

/// `radar replay publish` arguments.
#[derive(Debug, Parser)]
pub struct PublishArgs {
    /// Case id (`rolling-fork-2023` or `greenfield-2024`).
    #[arg(long)]
    pub case: String,
    /// The finished private revision to publish (its tiles are copied, never edited).
    #[arg(long)]
    pub revision: String,
    /// Warnings file written by `radar import-replay-warnings` for this case.
    #[arg(long)]
    pub warnings: std::path::PathBuf,
    /// Tiles root, normally `TILE_DIR`.
    #[arg(long, env = "TILE_DIR", default_value = "/data/tiles")]
    pub tile_root: std::path::PathBuf,
    /// Name of the public revision; defaults to `<revision>-public`.
    #[arg(long)]
    pub published_revision: Option<String>,
}

/// `radar replay check` arguments.
#[derive(Debug, Parser)]
pub struct CheckArgs {
    /// Case id (`rolling-fork-2023` or `greenfield-2024`).
    #[arg(long)]
    pub case: String,
    /// Anonymous MRMS bucket base URL.
    #[arg(
        long,
        env = "MRMS_BASE_URL",
        default_value = "https://noaa-mrms-pds.s3.amazonaws.com"
    )]
    pub base_url: String,
}

/// `radar replay import` arguments.
#[derive(Debug, Parser)]
pub struct ImportArgs {
    /// Case id (`rolling-fork-2023` or `greenfield-2024`).
    #[arg(long)]
    pub case: String,
    /// Revision name; defaults to the current UTC time (`YYYYMMDDtHHMMSSz`).
    #[arg(long)]
    pub revision: Option<String>,
    /// Tiles root, normally `TILE_DIR`. The import writes only under `<root>/replays/`.
    #[arg(long, env = "TILE_DIR", default_value = "/data/tiles")]
    pub tile_root: std::path::PathBuf,
    /// Pretend to run: validate, list objects and print the plan; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Render at most N frames per layer (pilot slices).
    #[arg(long)]
    pub max_frames: Option<usize>,
    /// Restrict to these layers (repeat the flag); default is every case layer.
    #[arg(long = "layer")]
    pub layers: Vec<String>,
    /// Verify cropped boundary PNG bytes against an uncropped render.
    #[arg(long)]
    pub verify_crop: bool,
    /// Anonymous MRMS bucket base URL.
    #[arg(
        long,
        env = "MRMS_BASE_URL",
        default_value = "https://noaa-mrms-pds.s3.amazonaws.com"
    )]
    pub base_url: String,
}

/// Run a replay subcommand.
pub async fn run(args: ReplayArgs) -> anyhow::Result<()> {
    match args.cmd {
        ReplayCmd::Check(args) => {
            let s3 =
                radar_grib::s3::S3Client::new(radar_grib::http::client()?, args.base_url.clone());
            let coverage = objects::check_case(&s3, &args.case).await?;
            println!("{}", serde_json::to_string_pretty(&coverage)?);
            Ok(())
        }
        ReplayCmd::Publish(args) => {
            let report = tokio::task::spawn_blocking(move || {
                publish::run(publish::Options {
                    case_id: args.case,
                    revision: args.revision,
                    warnings: args.warnings,
                    tile_root: args.tile_root,
                    published_revision: args.published_revision,
                })
            })
            .await??;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        ReplayCmd::Import(args) => {
            let report = import::run(import::Options {
                case_id: args.case,
                revision: args.revision,
                tile_root: args.tile_root,
                dry_run: args.dry_run,
                max_frames: args.max_frames,
                layers: args.layers,
                base_url: args.base_url,
                verify_crop: args.verify_crop,
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
    }
}
