//! Deployment smoke activity (spec section 9.2).
//!
//! The Python check imports pysteps and pygrib. The Rust build replaces them with native checks:
//! palette files resolve through `radar_tiler`, a 2x2 indexed PNG is recoloured and decoded, and
//! every volume is probed by a write, fsync, rename and read-back. The pysteps and ecCodes checks
//! have no native counterpart here and are not performed (see the crate README).

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use radar_types::payloads::smoke::DeploymentSmokeResult;
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::with_cancel;

/// Roles that the Python check accepts (`check_release`).
pub const KNOWN_ROLES: [&str; 7] = ["mrms", "nowcast", "hrrr", "aux", "alerts", "legacy", "all"];
/// Palettes checked when `PALETTES` is unset.
pub const DEFAULT_PALETTES: &str = "classic,muted,vivid";

/// Activity implementer for the deployment smoke check. Registered on every role.
#[derive(Default)]
pub struct SmokeActivities;

impl SmokeActivities {
    /// Create the implementer.
    pub fn new() -> Self {
        Self
    }
}

#[activities]
impl SmokeActivities {
    /// `radar_deployment_smoke`: run the release checks on blocking threads and return `{ok, role}`.
    #[activity(name = "radar_deployment_smoke")]
    pub async fn radar_deployment_smoke(
        self: Arc<Self>,
        ctx: ActivityContext,
    ) -> Result<DeploymentSmokeResult, ActivityError> {
        let task = tokio::task::spawn_blocking(|| check_release(&|name| std::env::var(name).ok()));
        with_cancel(&ctx, async { task.await.map_err(anyhow::Error::from)? }).await
    }
}

/// Run the release checks against an injectable environment lookup. Returns the Python result shape.
pub fn check_release(
    env: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<DeploymentSmokeResult> {
    let role = env("WORKER_ROLE")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "legacy".to_owned());
    if !KNOWN_ROLES.contains(&role.as_str()) {
        bail!("Unknown worker role");
    }

    let palettes = env("PALETTES")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_PALETTES.to_owned());
    for name in palettes.split(',').map(str::trim) {
        radar_tiler::load_palette(name)
            .with_context(|| format!("Configured palette is absent from the image: {name}"))?;
    }

    verify_png_round_trip()?;

    let volumes: Vec<(PathBuf, &str)> = if role == "alerts" {
        let grid_dir = env("GRID_DIR")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/data/grids".to_owned());
        check_readable_dir(Path::new(&grid_dir))?;
        vec![(
            PathBuf::from(
                env("STATE_DIR")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "/data/state".to_owned()),
            ),
            "STATE_DIR",
        )]
    } else {
        vec![
            (
                PathBuf::from(
                    env("TILE_DIR")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "/data/tiles".to_owned()),
                ),
                "TILE_DIR",
            ),
            (
                PathBuf::from(
                    env("GRID_DIR")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "/data/grids".to_owned()),
                ),
                "GRID_DIR",
            ),
            (
                PathBuf::from(
                    env("STATE_DIR")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "/data/state".to_owned()),
                ),
                "STATE_DIR",
            ),
        ]
    };
    for (dir, _label) in &volumes {
        probe_volume(dir)?;
    }

    Ok(DeploymentSmokeResult { ok: true, role })
}

/// Encode a 2x2 black/white indexed PNG, recolour black to red, and check pixel (1,0).
fn verify_png_round_trip() -> anyhow::Result<()> {
    // Indices: row 0 = [white, black], row 1 = [white, white].
    let tile = [1u8, 0, 1, 1];
    let plte = [0u8, 0, 0, 255, 255, 255];
    let png =
        radar_tiler::encode_indexed(&tile, 2, 2, &plte, &[], 6).context("encoding smoke PNG")?;
    let recoloured = radar_tiler::replace_palette(&png, &[255, 0, 0, 255, 255, 255], None)
        .context("replacing smoke palette")?;
    let decoded = radar_tiler::decode_indexed(&recoloured).context("decoding smoke PNG")?;
    let index = decoded
        .indices
        .get(1)
        .copied()
        .context("smoke PNG has no pixel (1,0)")?;
    let start = usize::from(index) * 3;
    let colour = decoded
        .plte
        .get(start..start + 3)
        .context("smoke PNG palette too short")?;
    if colour != [255, 0, 0] {
        bail!("Palette fixture produced incorrect pixels");
    }
    Ok(())
}

/// `os.scandir` one entry and require read and execute access (alerts grid directory).
fn check_readable_dir(dir: &Path) -> anyhow::Result<()> {
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading grid directory {}", dir.display()))?;
    let _ = entries.next().transpose()?;
    rustix::fs::access(
        dir,
        rustix::fs::Access::READ_OK | rustix::fs::Access::EXEC_OK,
    )
    .with_context(|| format!("grid directory {} is not readable", dir.display()))?;
    Ok(())
}

/// Probe one volume: the directory must exist (never created), then a PNG is written to a pending
/// file, fsynced, renamed to published and read back. Scratch files are removed on every path.
fn probe_volume(dir: &Path) -> anyhow::Result<()> {
    match std::fs::metadata(dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => bail!("volume {} is not a directory", dir.display()),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            bail!(
                "volume {} is not mounted (FileNotFoundError)",
                dir.display()
            )
        }
        Err(err) => return Err(err).with_context(|| format!("probing {}", dir.display())),
    }
    let scratch = tempfile::Builder::new()
        .prefix(".deployment-smoke-")
        .tempdir_in(dir)
        .with_context(|| format!("creating scratch directory in {}", dir.display()))?;
    let payload = smoke_png()?;
    let pending = scratch.path().join("pending.png");
    let published = scratch.path().join("published.png");
    {
        let mut file = File::create(&pending)?;
        file.write_all(&payload)?;
        file.sync_all()?;
    }
    std::fs::rename(&pending, &published)?;
    let mut back = Vec::new();
    File::open(&published)?.read_to_end(&mut back)?;
    if back != payload {
        bail!(
            "volume {} returned different bytes on read-back",
            dir.display()
        );
    }
    Ok(())
}

fn smoke_png() -> anyhow::Result<Vec<u8>> {
    let tile = [1u8, 0, 1, 1];
    let plte = [0u8, 0, 0, 255, 255, 255];
    radar_tiler::encode_indexed(&tile, 2, 2, &plte, &[], 6).context("encoding smoke PNG")
}
