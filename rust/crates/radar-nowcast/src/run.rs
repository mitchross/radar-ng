//! The `nowcast_run` activity body (spec-nowcast sections 2, 4 to 9).
//!
//! Phases, each CPU-bound step on the blocking pool with a heartbeat every
//! [`NowcastConfig::heartbeat_every`]:
//!
//! 1. setup: select inputs, validate, load (early exits write `warming_up` or `degraded`);
//! 2. pysteps: motion, S-PROG, and the dry-window check;
//! 3. render: one pyramid set and one point grid per lead, leads in parallel;
//! 4. commit: observed anchor, generation marker, manifest swap, retention, processed set, and only
//!    then `nowcast-status.json = ok` (PLAN D4);
//! 5. verification: score the observation against retained runs and summarise the log (DEC-8: the
//!    scoring has heartbeats too, and it runs after the commit so it never delays a frame).
//!
//! The function takes no Temporal types. The worker wraps it as an activity.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use radar_core::grid_dump::{
    GridInput, finalize_grid_generation, prune_grid_generations, write_grid,
};
use radar_core::heartbeat::run_blocking_with_heartbeat;
use radar_core::manifest::{FrameSpec, ReplaceOptions, replace_layer};
use radar_core::nowcast_skill::Summary;
use radar_core::processed_set::ProcessedSet;
use radar_core::pycompat::float_repr;
use radar_core::time::round_digits;
use radar_tiler::{ColorTable, load_palette};
use radar_types::payloads::nowcast::NowcastResult;
use serde_json::{Map, Value};

use crate::config::{MIN_INPUT_FRAMES, NowcastConfig};
use crate::error::NowcastError;
use crate::field::Field2;
use crate::inputs;
use crate::motion::{MotionParams, estimate_motion};
use crate::render::{AcceptedLead, NODATA, RenderContext, render_lead, resolution_km};
use crate::score::{self, OBSERVED_KEY};
use crate::select::{input_interval_min, lead_steps};
use crate::sprog::{self, SprogParams};
use crate::status::write_status;

/// Name of the layer the manifest publishes (`layers.nowcast`).
pub const LAYER: &str = "nowcast";
/// Processed-anchor file in the state directory.
pub const PROCESSED_FILE: &str = "nowcast.json";
/// Entries kept in the processed set (`ProcessedSet(max_entries=100)`).
pub const PROCESSED_MAX: usize = 100;
/// Method label of the S-PROG forecast (`pysteps-sprog`).
pub const METHOD_SPROG: &str = "pysteps-sprog";
/// Method label of the persistence fallback (DEC-7).
pub const METHOD_PERSISTENCE: &str = "persistence";

/// Result of one `nowcast_run` call: the payload and the verification summary for the metrics endpoint.
#[derive(Debug, Clone)]
pub struct NowcastOutput {
    /// The activity result (`NowcastResult`, spec 2.4).
    pub result: NowcastResult,
    /// The rolling skill summary after this run's scoring. `None` when the run did not commit.
    pub skill_summary: Option<Summary>,
    /// Prometheus text lines for the skill gauges (spec 10.6).
    pub prometheus: Vec<String>,
}

/// Run the nowcast once. See the module docs for the phases.
///
/// `heartbeat` is called about every `cfg.heartbeat_every` while CPU work runs, from a task that is not on
/// the rayon pool. A panicking callback is logged and never fails the run.
pub async fn nowcast_run(
    cfg: Arc<NowcastConfig>,
    heartbeat: Arc<dyn Fn() + Send + Sync>,
) -> anyhow::Result<NowcastOutput> {
    let started = Instant::now();
    let every = cfg.heartbeat_every;

    let setup = {
        let cfg = Arc::clone(&cfg);
        blocking(every, &heartbeat, move || setup(&cfg)).await?
    };
    let inputs = match setup {
        Setup::Skip(result) => {
            return Ok(NowcastOutput {
                result,
                skill_summary: None,
                prometheus: Vec::new(),
            });
        }
        Setup::Ready(inputs) => inputs,
    };
    let Inputs {
        latest,
        latest_dt,
        frames,
        newest,
        lats,
        lons,
        interval_min,
    } = inputs;
    let newest = Arc::new(newest);
    let steps = lead_steps(cfg.horizon_min, cfg.step_min, interval_min);
    let n_lead = steps.len();

    let forecast = {
        let cfg = Arc::clone(&cfg);
        let newest = Arc::clone(&newest);
        blocking(every, &heartbeat, move || {
            run_pysteps(&cfg, frames, &newest, &steps)
        })
        .await?
    };
    let (fields, no_rain, method) = match forecast {
        Forecasted::Stop => {
            return Ok(NowcastOutput {
                result: not_ran(Some(latest)),
                skill_summary: None,
                prometheus: Vec::new(),
            });
        }
        Forecasted::Fields {
            fields,
            no_rain,
            method,
        } => (fields, no_rain, method),
    };

    // Dry window (D2): nothing is rendered; the commit publishes an empty run and says so in the status.
    let (leads, palettes) = if no_rain {
        (Vec::new(), Vec::new())
    } else {
        let rendered = {
            let cfg = Arc::clone(&cfg);
            let latest = latest.clone();
            let lats = lats.clone();
            let lons = lons.clone();
            blocking(every, &heartbeat, move || {
                render_all(
                    &cfg,
                    &latest,
                    latest_dt,
                    &lats,
                    &lons,
                    method,
                    interval_min,
                    &fields,
                )
            })
            .await?
        };
        let leads: Vec<AcceptedLead> = rendered.into_iter().flatten().collect();
        let palettes: BTreeSet<String> = leads
            .iter()
            .flat_map(|l| l.palettes.iter().cloned())
            .collect();
        (leads, palettes.into_iter().collect::<Vec<_>>())
    };
    let leadtimes = if no_rain { 0 } else { n_lead as i64 };
    let palettes_out = palettes.clone();

    {
        let cfg = Arc::clone(&cfg);
        let latest = latest.clone();
        let lats = lats.clone();
        let lons = lons.clone();
        let newest = Arc::clone(&newest);
        blocking(every, &heartbeat, move || {
            commit_run(CommitPlan {
                cfg: &cfg,
                anchor: &latest,
                expected_leads: n_lead,
                leads,
                palettes,
                observed: &newest,
                lats: &lats,
                lons: &lons,
                method,
                no_rain,
            })
        })
        .await?;
    }

    let scored = {
        let cfg = Arc::clone(&cfg);
        let latest = latest.clone();
        let newest = Arc::clone(&newest);
        blocking(every, &heartbeat, move || {
            let entries = match score::score_observation(&cfg, &newest, &latest, &lats, &lons, Utc::now()) {
                Ok(entries) => entries.len(),
                Err(err) => {
                    tracing::warn!(valid = %latest, error = %format!("{err:#}"), "nowcast_skill_failed");
                    0
                }
            };
            let (summary, prometheus) = score::skill_summary(&cfg, Utc::now());
            Ok((entries, summary, prometheus))
        })
        .await?
    };
    let (skill_scored, summary, prometheus) = scored;

    let result = NowcastResult {
        ran: true,
        anchor_ts: Some(latest),
        leadtimes,
        palettes: palettes_out,
        duration_s: round_digits(started.elapsed().as_secs_f64(), 2),
        skill_scored: skill_scored as i64,
    };
    Ok(NowcastOutput {
        result,
        skill_summary: Some(summary),
        prometheus,
    })
}

/// Blocking work with a heartbeat. Errors and panics of `work` reach the caller.
async fn blocking<T, F>(
    every: Duration,
    beat: &Arc<dyn Fn() + Send + Sync>,
    work: F,
) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let beat = Arc::clone(beat);
    run_blocking_with_heartbeat(work, every, move || (*beat)()).await
}

fn not_ran(anchor: Option<String>) -> NowcastResult {
    NowcastResult {
        ran: false,
        anchor_ts: anchor,
        leadtimes: 0,
        palettes: Vec::new(),
        duration_s: 0.0,
        skill_scored: 0,
    }
}

enum Setup {
    /// Stop with this result (early exit, already processed, or warming up).
    Skip(NowcastResult),
    /// Inputs are valid. Continue with the forecast.
    Ready(Inputs),
}

struct Inputs {
    latest: String,
    latest_dt: DateTime<Utc>,
    frames: Vec<Field2<f32>>,
    newest: Field2<f32>,
    lats: Vec<f64>,
    lons: Vec<f64>,
    interval_min: f64,
}

/// Steps 1 to 8 of spec 4.3 (`_setup`). Writes `warming_up` and `degraded` statuses where Python does.
fn setup(cfg: &NowcastConfig) -> anyhow::Result<Setup> {
    std::fs::create_dir_all(&cfg.paths.tile_dir)?;
    std::fs::create_dir_all(&cfg.paths.state_dir)?;
    let candidates = inputs::recent_input_grids(cfg);
    if candidates.len() < MIN_INPUT_FRAMES {
        tracing::info!(count = candidates.len(), "waiting_for_grids");
        write_status(
            &cfg.paths.state_dir,
            "warming_up",
            Some("insufficient_inputs"),
            Some(&format!(
                "count={},required={MIN_INPUT_FRAMES}",
                candidates.len()
            )),
        )?;
        return Ok(Setup::Skip(not_ran(None)));
    }
    let latest_stem = candidates.last().expect("non-empty").stem.clone();
    if is_processed(cfg, &latest_stem)? {
        return Ok(Setup::Skip(not_ran(Some(latest_stem))));
    }

    let mut loaded = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let Some(at) = candidate.time else {
            continue;
        };
        if let Some(grid) = inputs::load_grid(&candidate.item) {
            loaded.push((grid, at, candidate.stem.clone()));
        }
    }
    if loaded.len() < MIN_INPUT_FRAMES {
        write_status(
            &cfg.paths.state_dir,
            "warming_up",
            Some("insufficient_valid_inputs"),
            Some(&format!(
                "count={},required={MIN_INPUT_FRAMES}",
                loaded.len()
            )),
        )?;
        return Ok(Setup::Skip(not_ran(Some(latest_stem))));
    }
    let (w, h) = (
        loaded[loaded.len() - 1].0.values.width(),
        loaded[loaded.len() - 1].0.values.height(),
    );
    loaded.retain(|(g, _, _)| g.values.width() == w && g.values.height() == h);
    if loaded.len() < MIN_INPUT_FRAMES {
        write_status(
            &cfg.paths.state_dir,
            "warming_up",
            Some("inconsistent_input_shapes"),
            Some(&format!(
                "count={},required={MIN_INPUT_FRAMES}",
                loaded.len()
            )),
        )?;
        return Ok(Setup::Skip(not_ran(Some(latest_stem))));
    }
    let latest = loaded[loaded.len() - 1].2.clone();
    if is_processed(cfg, &latest)? {
        return Ok(Setup::Skip(not_ran(Some(latest))));
    }

    let times: Vec<DateTime<Utc>> = loaded.iter().map(|(_, t, _)| *t).collect();
    let interval_min = match input_interval_min(&times, cfg.max_input_gap_min, cfg.input_step_min) {
        Ok(m) => m,
        Err(intervals) => {
            let detail = format!(
                "intervals_minutes=[{}]",
                intervals
                    .iter()
                    .map(|v| float_repr(*v))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            write_status(
                &cfg.paths.state_dir,
                "degraded",
                Some("invalid_input_cadence"),
                Some(&detail),
            )?;
            return Ok(Setup::Skip(not_ran(Some(latest))));
        }
    };

    let (newest_grid, latest_dt, _) = loaded.pop().expect("at least three grids");
    let newest = newest_grid.values.clone();
    let mut frames: Vec<Field2<f32>> = loaded.into_iter().map(|(g, _, _)| g.values).collect();
    frames.push(newest_grid.values);
    Ok(Setup::Ready(Inputs {
        latest,
        latest_dt,
        frames,
        newest,
        lats: newest_grid.lats,
        lons: newest_grid.lons,
        interval_min,
    }))
}

fn is_processed(cfg: &NowcastConfig, stem: &str) -> anyhow::Result<bool> {
    let set = ProcessedSet::open(cfg.paths.state_file(PROCESSED_FILE), PROCESSED_MAX)?;
    Ok(set.contains(stem))
}

enum Forecasted {
    /// The forecast failed with no fallback. The run stops with `ran = false`.
    Stop,
    /// Forecast fields, in lead order.
    Fields {
        fields: Vec<Field2<f32>>,
        no_rain: bool,
        method: &'static str,
    },
}

/// Motion and S-PROG (spec 5 to 7). A failure writes `degraded / pysteps_failed` (spec 9.3).
fn run_pysteps(
    cfg: &NowcastConfig,
    mut frames: Vec<Field2<f32>>,
    newest: &Field2<f32>,
    steps: &[f64],
) -> anyhow::Result<Forecasted> {
    // Values below -100 are missing (spec 5). The newest raw frame is kept separately for the observed grid.
    for frame in &mut frames {
        frame.mask_below(-100.0);
    }
    let attempt = (|| -> Result<sprog::Forecast, NowcastError> {
        let uv = estimate_motion(&frames, cfg.motion_stride, &MotionParams::radar_ng())?;
        sprog::forecast(&frames, &uv, steps, &SprogParams::radar_ng())
    })();
    match attempt {
        Ok(fc) => Ok(Forecasted::Fields {
            fields: fc.fields,
            no_rain: fc.no_rain,
            method: METHOD_SPROG,
        }),
        Err(err) => {
            tracing::warn!(error = %err, "pysteps_failed");
            write_status(
                &cfg.paths.state_dir,
                "degraded",
                Some("pysteps_failed"),
                Some(&err.to_string()),
            )?;
            if cfg.allow_persistence_fallback {
                let fields = steps.iter().map(|_| newest.clone()).collect();
                Ok(Forecasted::Fields {
                    fields,
                    no_rain: false,
                    method: METHOD_PERSISTENCE,
                })
            } else {
                Ok(Forecasted::Stop)
            }
        }
    }
}

/// Palettes that have a `reflectivity` table (spec 8.3 and `_load_palette_tables`).
struct PaletteSet {
    tables: Vec<(String, ColorTable)>,
    expected: BTreeSet<String>,
}

fn load_palette_set(cfg: &NowcastConfig) -> PaletteSet {
    let mut tables: Vec<(String, ColorTable)> = Vec::new();
    let mut names: Vec<String> = cfg.palettes.clone();
    names.dedup();
    for name in names {
        if tables.iter().any(|(n, _)| *n == name) {
            continue;
        }
        match load_reflectivity(&name) {
            Ok(Some(table)) => tables.push((name, table)),
            Ok(None) => {}
            Err(err) => tracing::warn!(palette = %name, error = %err, "palette_missing"),
        }
    }
    if tables.is_empty()
        && let Ok(Some(table)) = load_reflectivity("classic")
    {
        tables.push(("classic".to_string(), table));
    }
    let expected = tables.iter().map(|(n, _)| n.clone()).collect();
    PaletteSet { tables, expected }
}

fn load_reflectivity(name: &str) -> anyhow::Result<Option<ColorTable>> {
    let palette = load_palette(name)?;
    if palette.color_keys().any(|k| k == "reflectivity") {
        Ok(Some(palette.color_table("reflectivity")?))
    } else {
        Ok(None)
    }
}

/// Render leads in order, with `render_workers` threads parallelizing each lead's tiles.
#[allow(clippy::too_many_arguments)]
fn render_all(
    cfg: &NowcastConfig,
    latest: &str,
    latest_dt: DateTime<Utc>,
    lats: &[f64],
    lons: &[f64],
    method: &'static str,
    interval_min: f64,
    fields: &[Field2<f32>],
) -> anyhow::Result<Vec<Option<AcceptedLead>>> {
    let set = load_palette_set(cfg);
    if set.expected.is_empty() {
        bail!("no palette with a reflectivity table is loaded");
    }
    let ctx = RenderContext {
        cfg,
        tables: &set.tables,
        expected: &set.expected,
        anchor: latest,
        latest_dt,
        lats,
        lons,
        method,
        input_interval_min: interval_min,
        resolution_km: resolution_km(lats),
    };
    render_pool(cfg.render_workers)?.install(|| {
        // Nested lead tasks can block on publication locks held by suspended tile tasks in this pool.
        (0..fields.len())
            .map(|i| render_lead(&ctx, i, &fields[i]))
            .collect::<anyhow::Result<Vec<_>>>()
    })
}

/// One render pool per thread count, kept for the life of the process instead of new threads every run.
fn render_pool(threads: usize) -> anyhow::Result<Arc<rayon::ThreadPool>> {
    static POOLS: Mutex<BTreeMap<usize, Arc<rayon::ThreadPool>>> = Mutex::new(BTreeMap::new());
    let mut pools = POOLS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(pool) = pools.get(&threads) {
        return Ok(Arc::clone(pool));
    }
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("nowcast-render-{i}"))
            .build()
            .context("building the render pool")?,
    );
    pools.insert(threads, Arc::clone(&pool));
    Ok(pool)
}

/// Inputs to [`commit_run`]. `leads` must hold every lead for a non-dry run.
pub(crate) struct CommitPlan<'a> {
    /// Run configuration.
    pub cfg: &'a NowcastConfig,
    /// Anchor timestamp, the run id.
    pub anchor: &'a str,
    /// Leads the run must render (spec 8.8).
    pub expected_leads: usize,
    /// Accepted leads in lead order.
    pub leads: Vec<AcceptedLead>,
    /// Sorted union of the rendered palettes.
    pub palettes: Vec<String>,
    /// The newest raw science grid (stored as the observed anchor).
    pub observed: &'a Field2<f32>,
    /// Science-grid latitudes, descending.
    pub lats: &'a [f64],
    /// Science-grid longitudes, ascending.
    pub lons: &'a [f64],
    /// Method label of the forecast.
    pub method: &'static str,
    /// True for a dry window: no leads, and the layer is removed.
    pub no_rain: bool,
}

/// Commit a run (spec 9.5). Order: observed anchor, generation marker, manifest swap, retention,
/// processed set, and last the status `ok` (D4). Any failure before the status leaves the status file
/// unchanged, so nothing reports `ok` for a run that did not commit.
pub(crate) fn commit_run(plan: CommitPlan<'_>) -> anyhow::Result<()> {
    let CommitPlan {
        cfg,
        anchor,
        expected_leads,
        leads,
        palettes,
        observed,
        lats,
        lons,
        method,
        no_rain,
    } = plan;
    if !no_rain && leads.len() != expected_leads {
        bail!(
            "nowcast incomplete: rendered {}/{} leadtimes",
            leads.len(),
            expected_leads
        );
    }
    let observed_key = format!("runs/{anchor}/{OBSERVED_KEY}");
    let stored = write_grid(
        &cfg.grid,
        LAYER,
        anchor,
        &GridInput {
            data: observed.as_slice(),
            lats,
            lons,
            unit: "dBZ",
            fill: f64::from(NODATA),
            max_cells: Some(cfg.point_grid_max_cells),
            grid_key: Some(&observed_key),
        },
    );
    if let Err(err) = stored {
        tracing::warn!(anchor, error = %err, "nowcast_observed_anchor_failed");
    }
    finalize_grid_generation(&cfg.grid, LAYER, anchor)
        .context("finalizing the nowcast generation")?;

    let timestamps: Vec<String> = leads.iter().map(|l| l.timestamp.clone()).collect();
    let frames: Vec<FrameSpec> = leads.into_iter().map(|l| l.frame).collect();
    let mut layer_metadata = Map::new();
    layer_metadata.insert("title".into(), Value::from("MRMS motion nowcast"));
    layer_metadata.insert("kind".into(), Value::from("nowcast"));
    layer_metadata.insert("horizon_minutes".into(), Value::from(cfg.horizon_min));
    layer_metadata.insert("step_minutes".into(), Value::from(cfg.step_min));
    layer_metadata.insert("method".into(), Value::from(method));
    layer_metadata.insert("run_id".into(), Value::from(anchor));
    let options = ReplaceOptions {
        palettes,
        frames,
        layer_metadata: Some(layer_metadata),
    };
    replace_layer(&cfg.paths.state_dir, LAYER, &timestamps, &options)
        .context("replacing the nowcast manifest layer")?;

    prune_grid_generations(&cfg.grid, LAYER, cfg.retention_runs, Some(anchor))
        .context("pruning nowcast runs")?;
    let mut processed = ProcessedSet::open(cfg.paths.state_file(PROCESSED_FILE), PROCESSED_MAX)?;
    processed.add(anchor)?;

    // Only a committed run that used the forecast reports ok. A persistence fallback stays degraded.
    if method == METHOD_SPROG {
        let written = if no_rain {
            write_status(
                &cfg.paths.state_dir,
                "ok",
                Some("no_rain"),
                Some("no cell above 5 dBZ in the input window"),
            )
        } else {
            write_status(&cfg.paths.state_dir, "ok", None, None)
        };
        if let Err(err) = written {
            tracing::warn!(anchor, error = %err, "nowcast_status_write_failed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ZOOM_LEVELS;
    use radar_core::env::Paths;
    use radar_core::grid_dump::GridConfig;

    fn cfg_in(dir: &std::path::Path) -> NowcastConfig {
        let paths = Paths::new(dir.join("state"), dir.join("grids"), dir.join("tiles"));
        let mut cfg = NowcastConfig::with_paths(paths);
        cfg.grid = GridConfig::with_dir(dir.join("grids"));
        cfg
    }

    /// §13 item 15 (commit path) and D4: a run with missing leads fails and leaves no status, no manifest and
    /// no processed entry behind.
    #[test]
    fn incomplete_commit_fails_and_writes_no_status() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_in(dir.path());
        let observed = Field2::filled(8, 6, NODATA);
        let lats = crate::inputs::linspace(54.0, 50.0, 6);
        let lons = crate::inputs::linspace(-100.0, -95.0, 8);
        let err = commit_run(CommitPlan {
            cfg: &cfg,
            anchor: "2026-07-15T16:00:00+00:00",
            expected_leads: 12,
            leads: Vec::new(),
            palettes: Vec::new(),
            observed: &observed,
            lats: &lats,
            lons: &lons,
            method: METHOD_SPROG,
            no_rain: false,
        })
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("nowcast incomplete: rendered 0/12 leadtimes"),
            "{err}"
        );
        assert!(
            !cfg.paths
                .state_dir
                .join(crate::status::STATUS_FILE)
                .exists()
        );
        assert!(!cfg.paths.state_dir.join("manifest.json").exists());
        assert!(!cfg.paths.state_dir.join(PROCESSED_FILE).exists());
    }

    /// A persistence fallback commits but does not claim `ok`.
    #[test]
    fn persistence_commit_keeps_status_degraded() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_in(dir.path());
        write_status(
            &cfg.paths.state_dir,
            "degraded",
            Some("pysteps_failed"),
            Some("x"),
        )
        .unwrap();
        let observed = Field2::filled(8, 6, NODATA);
        let lats = crate::inputs::linspace(54.0, 50.0, 6);
        let lons = crate::inputs::linspace(-100.0, -95.0, 8);
        commit_run(CommitPlan {
            cfg: &cfg,
            anchor: "2026-07-15T16:00:00+00:00",
            expected_leads: 0,
            leads: Vec::new(),
            palettes: Vec::new(),
            observed: &observed,
            lats: &lats,
            lons: &lons,
            method: METHOD_PERSISTENCE,
            no_rain: true,
        })
        .unwrap();
        let text =
            std::fs::read_to_string(cfg.paths.state_dir.join(crate::status::STATUS_FILE)).unwrap();
        assert!(text.contains("\"status\":\"degraded\""), "{text}");
    }

    #[test]
    fn retention_floor_is_two_runs() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_in(dir.path());
        assert!(cfg.retention_runs >= 2);
        assert_eq!(ZOOM_LEVELS.len(), 3);
    }

    #[test]
    fn render_many_leads_with_shared_publication_lock_stripes() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = cfg_in(dir.path());
        cfg.render_workers = 4;
        cfg.step_min = 1;
        let fields = vec![Field2::filled(32, 32, 30.0); 72];
        let lats = inputs::linspace(45.0, 40.0, 32);
        let lons = inputs::linspace(-90.0, -80.0, 32);
        let anchor = "2026-10-15T02:00:00+00:00";
        let leads = render_all(
            &cfg,
            anchor,
            inputs::stem_time(anchor).unwrap(),
            &lats,
            &lons,
            METHOD_SPROG,
            5.0,
            &fields,
        )
        .unwrap();
        assert_eq!(leads.len(), fields.len());
        assert!(leads.iter().all(Option::is_some));
    }
}
