//! Deployment smoke activity (spec section 9.2).
//!
//! The original Python check imported pysteps and pygrib. The Rust build uses native checks instead:
//! palette files resolve through `radar_tiler`, a 2x2 indexed PNG is recoloured and decoded, and
//! every volume is probed by a write, fsync, rename and read-back. The Level II pool has no data
//! volumes, so it probes only its `TMPDIR` scratch volume. The pysteps and ecCodes checks
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

/// Roles that `check_release` accepts.
pub const KNOWN_ROLES: [&str; 8] = [
    "mrms", "nowcast", "hrrr", "aux", "alerts", "level2", "legacy", "all",
];
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

/// Run the release checks against an injectable environment lookup. Returns `{ok, role}`.
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

    let truthy = |key: &str| {
        env(key).is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
    };
    let non_empty = |key: &str| env(key).is_some_and(|v| !v.trim().is_empty());
    let state_pg_active = non_empty("RADAR_STATE_DATABASE_URL") && truthy("RADAR_STATE_READ_PG");
    let s3_configured = non_empty("RADAR_S3_ENDPOINT");
    let grids_s3_active = s3_configured && truthy("RADAR_GRIDS_READ_S3");
    let tiles_s3_active = s3_configured && truthy("RADAR_TILES_READ_S3");

    let mut volumes: Vec<(PathBuf, &str)> = Vec::new();
    if role == "level2" {
        volumes.push((
            PathBuf::from(
                env("TMPDIR")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "/scratch".to_owned()),
            ),
            "TMPDIR",
        ));
    } else if role == "alerts" {
        if !grids_s3_active {
            let grid_dir = env("GRID_DIR")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/data/grids".to_owned());
            check_readable_dir(Path::new(&grid_dir))?;
        }
        if !state_pg_active {
            volumes.push((
                PathBuf::from(
                    env("STATE_DIR")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "/data/state".to_owned()),
                ),
                "STATE_DIR",
            ));
        }
    } else {
        if !tiles_s3_active {
            volumes.push((
                PathBuf::from(
                    env("TILE_DIR")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "/data/tiles".to_owned()),
                ),
                "TILE_DIR",
            ));
        }
        if !grids_s3_active {
            volumes.push((
                PathBuf::from(
                    env("GRID_DIR")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "/data/grids".to_owned()),
                ),
                "GRID_DIR",
            ));
        }
        if !state_pg_active {
            volumes.push((
                PathBuf::from(
                    env("STATE_DIR")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "/data/state".to_owned()),
                ),
                "STATE_DIR",
            ));
        }
    }
    if volumes.is_empty() {
        let scratch = env("TMPDIR")
            .filter(|s| !s.is_empty())
            .map_or_else(std::env::temp_dir, PathBuf::from);
        volumes.push((scratch, "TMPDIR"));
    }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use radar_types::wire::{self, WorkerRole};

    use super::*;
    use crate::worker::registration;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn level2_passes_with_only_tmpdir() {
        let scratch = tempfile::tempdir().unwrap();
        let missing = scratch.path().join("absent");
        let missing = missing.to_str().unwrap();
        let env = lookup(&[
            ("WORKER_ROLE", "level2"),
            ("TMPDIR", scratch.path().to_str().unwrap()),
            ("TILE_DIR", missing),
            ("GRID_DIR", missing),
            ("STATE_DIR", missing),
        ]);
        let result = check_release(&env).unwrap();
        assert!(result.ok);
        assert_eq!(result.role, "level2");
    }

    #[test]
    fn level2_fails_when_tmpdir_is_not_mounted() {
        let scratch = tempfile::tempdir().unwrap();
        let missing = scratch.path().join("absent");
        let env = lookup(&[
            ("WORKER_ROLE", "level2"),
            ("TMPDIR", missing.to_str().unwrap()),
        ]);
        let err = check_release(&env).unwrap_err().to_string();
        assert!(err.contains("not mounted"), "{err}");
    }

    #[test]
    fn other_roles_still_probe_data_volumes() {
        let scratch = tempfile::tempdir().unwrap();
        let missing = scratch.path().join("absent");
        let env = lookup(&[
            ("WORKER_ROLE", "mrms"),
            ("TMPDIR", scratch.path().to_str().unwrap()),
            ("TILE_DIR", missing.to_str().unwrap()),
            ("GRID_DIR", scratch.path().to_str().unwrap()),
            ("STATE_DIR", scratch.path().to_str().unwrap()),
        ]);
        assert!(check_release(&env).is_err());
    }

    #[test]
    fn volumeless_worker_skips_pg_and_s3_backed_dirs_and_probes_tmpdir() {
        let scratch = tempfile::tempdir().unwrap();
        let missing = scratch.path().join("absent");
        let missing = missing.to_str().unwrap();
        let env = lookup(&[
            ("WORKER_ROLE", "mrms"),
            ("TMPDIR", scratch.path().to_str().unwrap()),
            ("TILE_DIR", missing),
            ("GRID_DIR", missing),
            ("STATE_DIR", missing),
            ("RADAR_STATE_DATABASE_URL", "postgres://radar@pg/db"),
            ("RADAR_STATE_READ_PG", "1"),
            ("RADAR_S3_ENDPOINT", "http://rustfs:9000"),
            ("RADAR_GRIDS_READ_S3", "1"),
            ("RADAR_TILES_READ_S3", "1"),
        ]);
        let result = check_release(&env).unwrap();
        assert!(result.ok);
        assert_eq!(result.role, "mrms");
    }

    #[test]
    fn shadow_mode_still_probes_disk_volumes() {
        let scratch = tempfile::tempdir().unwrap();
        let missing = scratch.path().join("absent");
        let env = lookup(&[
            ("WORKER_ROLE", "mrms"),
            ("TMPDIR", scratch.path().to_str().unwrap()),
            ("TILE_DIR", missing.to_str().unwrap()),
            ("GRID_DIR", scratch.path().to_str().unwrap()),
            ("STATE_DIR", scratch.path().to_str().unwrap()),
            ("RADAR_STATE_DATABASE_URL", "postgres://radar@pg/db"),
            ("RADAR_STATE_READ_PG", "0"),
            ("RADAR_S3_ENDPOINT", "http://rustfs:9000"),
            ("RADAR_GRIDS_READ_S3", "0"),
            ("RADAR_TILES_READ_S3", "0"),
        ]);
        assert!(check_release(&env).is_err());
    }

    #[test]
    fn unknown_role_is_rejected() {
        let env = lookup(&[("WORKER_ROLE", "bogus")]);
        let err = check_release(&env).unwrap_err().to_string();
        assert_eq!(err, "Unknown worker role");
    }

    #[test]
    fn every_role_that_registers_smoke_is_known() {
        for role in WorkerRole::ALL {
            if registration(role)
                .activities
                .contains(&wire::ACTIVITY_RADAR_DEPLOYMENT_SMOKE)
            {
                assert!(
                    KNOWN_ROLES.contains(&role.as_str()),
                    "{} registers smoke but check_release rejects it",
                    role.as_str()
                );
            }
        }
    }
}
