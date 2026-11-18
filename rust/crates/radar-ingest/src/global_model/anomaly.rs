//! `temp-anomaly`: the GFS forecast daily high minus the 1991-2020 normal high, days 1 to 7, one frame a
//! day (design `weather-extras.md` section 6).
//!
//! A day is 06Z to 06Z. Its high is the cell-wise maximum of the four 6-hour `TMAX` records that end
//! at +6, +12, +18 and +24 h. It is compared with the normal *high* of that date, never with one
//! hourly reading or the daily mean. Each frame is stamped 18Z of its date, so every US time zone reads
//! it as that day. Cells without a normal (water, outside the lower 48) stay empty.

use std::time::Instant;

use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Timelike, Utc};
use radar_aux::static_data::normals::{Normals, Var};
use radar_core::grid_dump::{self, GridInput};
use radar_geo::{Grid, GridGeo};
use radar_grib::decode::{MessageSelector, SURFACE_HEIGHT_ABOVE_GROUND, find_message};
use radar_grib::hrrr::{Fetched, fetch_subset};
use radar_grib::mrms::ensure_south_up;
use radar_tiler::palettes::Palette;
use radar_types::payloads::hrrr::ForecastHourResult;

use super::Product;
use super::activities::{already_rendered, render};
use super::state::{self, CONUS_BBOX};
use crate::common::{self, elapsed_s};
use crate::config::IngestConfig;
use crate::ctx::IngestCtx;
use crate::heartbeat::{self, Heartbeat};
use crate::hrrr::render::{load_hrrr_palettes, tables_for_key};
use crate::hrrr::{HRRR_BEAT, run_init, valid_time};

/// Forecast days rendered.
pub const DAYS: u32 = 7;
/// Hour of the day boundary, UTC.
pub const DAY_START_UTC: u32 = 6;
/// The last record any run needs (a 12Z run: the next 06Z is +18 h, then 7 days). GFS posts hours in
/// order, so this index existing means the run has every record.
pub const PROBE_FHR: u32 = 18 + 24 * DAYS;

/// GRIB2 selector of the 2 m maximum temperature (0.0.4).
pub fn tmax_selector() -> MessageSelector {
    MessageSelector::parameter(0, 0, 4).at_level(SURFACE_HEIGHT_ABOVE_GROUND, 2.0)
}

/// `.idx` terms of the 6-hour `TMAX` record ending at `fhr`.
pub fn tmax_terms(fhr: u32) -> [String; 2] {
    [
        "TMAX:2 m above ground".to_string(),
        format!(":{}-{fhr} hour max fcst:", fhr - 6),
    ]
}

/// Hours from `init` to the first 06Z at or after it.
fn first_day_offset(init: DateTime<Utc>) -> u32 {
    (DAY_START_UTC + 24 - init.hour()) % 24
}

/// The frame hours of a run: 18Z of each of the [`DAYS`] days, as hours from `init`.
pub fn day_hours(init: DateTime<Utc>) -> Vec<u32> {
    let first = first_day_offset(init);
    (0..DAYS).map(|k| first + 24 * k + 12).collect()
}

/// The four record hours whose 6-hour maxima make the high of the day framed at `frame_fhr`.
pub fn window_records(frame_fhr: u32) -> [u32; 4] {
    [frame_fhr - 6, frame_fhr, frame_fhr + 6, frame_fhr + 12]
}

/// The calendar date of a frame (its 06Z start).
pub fn frame_date(init: DateTime<Utc>, frame_fhr: u32) -> NaiveDate {
    (init + ChronoDuration::hours(i64::from(frame_fhr))).date_naive()
}

/// Cell-wise maximum of the 6-hour maxima: the daily high. NaN when any record is missing there.
pub fn daily_high(records: &[Grid]) -> anyhow::Result<Grid> {
    let first = records
        .first()
        .ok_or_else(|| anyhow::anyhow!("no TMAX records"))?;
    let mut out = first.clone();
    for g in &records[1..] {
        anyhow::ensure!(
            (g.height, g.width) == (out.height, out.width),
            "TMAX grids differ in shape"
        );
        for (o, &v) in out.values.iter_mut().zip(&g.values) {
            *o = if o.is_nan() || v.is_nan() {
                f32::NAN
            } else {
                o.max(v)
            };
        }
    }
    Ok(out)
}

/// Kelvin to degrees F.
pub fn k_to_f(k: f32) -> f32 {
    (k - 273.15) * 1.8 + 32.0
}

/// Degrees C to degrees F.
pub fn c_to_f(c: f64) -> f64 {
    c * 1.8 + 32.0
}

/// Mean normal of the store cells inside one model cell, or `None` when fewer than half are land.
fn cell_normal(n: &Normals, plane: &[i16], lat: f64, lon: f64, half: f64) -> Option<f64> {
    let g = n.meta.grid;
    let steps = (half / g.step).floor() as i64;
    let (mut sum, mut valid, mut total) = (0.0, 0u32, 0u32);
    for dr in -steps..=steps {
        for dc in -steps..=steps {
            total += 1;
            let la = lat + dr as f64 * g.step;
            let lo = lon + dc as f64 * g.step;
            if let Some(v) = n.sample(plane, la, lo) {
                sum += v;
                valid += 1;
            }
        }
    }
    (valid * 2 >= total && valid > 0).then(|| sum / f64::from(valid))
}

/// Forecast high (Kelvin, on the cropped model grid) minus the normal high plane, degrees F.
pub fn anomaly(high_k: &Grid, normals: &Normals, normal_high: &[i16]) -> anyhow::Result<Grid> {
    let GridGeo::Regular { lats, lons } = &high_k.geo else {
        anyhow::bail!("temp-anomaly needs a regular grid");
    };
    let half = lats
        .windows(2)
        .next()
        .map_or(0.125, |w| (w[1] - w[0]).abs() / 2.0);
    let mut values = Vec::with_capacity(high_k.values.len());
    for (r, &lat) in lats.iter().enumerate() {
        for (c, &lon) in lons.iter().enumerate() {
            let k = high_k.at(r, c);
            values.push(match cell_normal(normals, normal_high, lat, lon, half) {
                Some(normal_c) if k.is_finite() => k_to_f(k) - c_to_f(normal_c) as f32,
                _ => f32::NAN,
            });
        }
    }
    Ok(Grid {
        width: high_k.width,
        height: high_k.height,
        values,
        geo: high_k.geo.clone(),
    })
}

/// Decode and crop one record's `TMAX`.
fn decode_tmax(bytes: &[u8]) -> anyhow::Result<Grid> {
    let mut msg = find_message(bytes, &tmax_selector())?
        .ok_or_else(|| anyhow::anyhow!("TMAX message missing from GRIB"))?;
    ensure_south_up(&mut msg.grid);
    state::crop(&msg.grid, CONUS_BBOX)
}

/// Whether the normals store is complete (its marker exists). Without it there is nothing to compare.
pub async fn normals_ready(cfg: &IngestConfig) -> anyhow::Result<bool> {
    let state_dir = cfg.paths.state_dir.clone();
    common::offload(move || Ok::<_, anyhow::Error>(Normals::open(&state_dir)?.is_some())).await?
}

/// Build, render and dump one day from its four records. Returns the layers rendered.
pub fn process_day_sync(
    cfg: &IngestConfig,
    run_id: &str,
    valid: &str,
    date: NaiveDate,
    records: &[Vec<u8>],
    palettes: &[Palette],
) -> anyhow::Result<Vec<String>> {
    let product = Product::TempAnomaly;
    let Some(normals) = Normals::open(&cfg.paths.state_dir)? else {
        tracing::info!(run_id = %run_id, "temp_anomaly_no_normals");
        return Ok(Vec::new());
    };
    let grids: Vec<Grid> = records
        .iter()
        .map(|b| decode_tmax(b))
        .collect::<anyhow::Result<_>>()?;
    let high = daily_high(&grids)?;
    let plane = normals.plane(Var::Tmax, date)?;
    let grid = anomaly(&high, &normals, &plane)?;
    let tables = tables_for_key(palettes, product.color_key());
    if !render(cfg, product, run_id, valid, &grid, &tables, None)? {
        return Ok(Vec::new());
    }
    let (lats, lons) = common::regular_axes(&grid)?;
    let input = GridInput::new(&grid.values, lats, lons, product.unit());
    if let Err(err) = grid_dump::write_grid(&cfg.grid, product.layer(), valid, &input) {
        tracing::warn!(err = %err, layer = product.layer(), "grid_dump_failed");
    }
    Ok(vec![product.layer().to_string()])
}

/// Render the day framed at `fhr`. Missing records or normals return no layers (the run stays
/// incomplete and is retried on the next poll).
pub async fn process_day(
    ctx: &IngestCtx,
    run_id: &str,
    fhr: i64,
    hb: &Heartbeat,
) -> anyhow::Result<ForecastHourResult> {
    let product = Product::TempAnomaly;
    let started = Instant::now();
    let init = run_init(run_id)?;
    let fhr_u32 = u32::try_from(fhr)
        .ok()
        .filter(|h| *h >= 6)
        .ok_or_else(|| anyhow::anyhow!("fhr out of range: {fhr}"))?;
    let valid = valid_time(init, fhr_u32);
    let result = |layers: Vec<String>| ForecastHourResult {
        fhr,
        rendered_layers: layers,
        duration_s: elapsed_s(started),
        valid_timestamp: valid.clone(),
    };
    let palettes = load_hrrr_palettes(&ctx.cfg.palettes);
    let done = {
        let (cfg, palettes) = (ctx.cfg.clone(), palettes.clone());
        let (run, v) = (run_id.to_string(), valid.clone());
        common::offload(move || already_rendered(&cfg, product, &palettes, &run, &v)).await?
    };
    if done {
        return Ok(result(vec![product.layer().to_string()]));
    }
    if !normals_ready(&ctx.cfg).await? {
        return Ok(result(Vec::new()));
    }

    let mut records = Vec::with_capacity(4);
    for rec in window_records(fhr_u32) {
        let details = serde_json::json!({"phase": "download", "layer": product.layer(), "fhr": rec, "run_id": run_id});
        hb(details.clone());
        let terms = tmax_terms(rec);
        let matcher: Vec<&str> = terms.iter().map(String::as_str).collect();
        let key = radar_grib::gfs::file_key(init, rec);
        let matchers = [matcher.as_slice()];
        let fetch = fetch_subset(&ctx.gfs_s3, &key, &matchers);
        match heartbeat::during(HRRR_BEAT, hb, details, fetch).await? {
            Some(Fetched::Data { bytes, .. }) => records.push(bytes.to_vec()),
            _ => {
                tracing::info!(layer = product.layer(), run_id = %run_id, fhr = rec, "record_pending");
                return Ok(result(Vec::new()));
            }
        }
    }

    let details = serde_json::json!({"phase": "render", "layer": product.layer(), "fhr": fhr, "run_id": run_id});
    let cfg = ctx.cfg.clone();
    let (run, render_valid) = (run_id.to_string(), valid.clone());
    let date = frame_date(init, fhr_u32);
    let rendered = heartbeat::blocking(HRRR_BEAT, hb, details, move || {
        process_day_sync(&cfg, &run, &render_valid, date, &records, &palettes)
    })
    .await?;
    let out = result(rendered);
    tracing::info!(layer = product.layer(), run_id = %run_id, fhr, rendered = ?out.rendered_layers, duration_s = out.duration_s, "day_done");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn init(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 18, h, 0, 0).unwrap()
    }

    #[test]
    fn days_run_06z_to_06z_and_frames_sit_at_18z() {
        assert_eq!(day_hours(init(0)), [18, 42, 66, 90, 114, 138, 162]);
        assert_eq!(day_hours(init(6)), [12, 36, 60, 84, 108, 132, 156]);
        assert_eq!(day_hours(init(12)), [30, 54, 78, 102, 126, 150, 174]);
        assert_eq!(day_hours(init(18)), [24, 48, 72, 96, 120, 144, 168]);
        for h in [0, 6, 12, 18] {
            let hours = day_hours(init(h));
            let last = *window_records(*hours.last().unwrap()).last().unwrap();
            assert!(last <= PROBE_FHR, "{h}z needs f{last}");
            for fhr in hours {
                let at = init(h) + ChronoDuration::hours(i64::from(fhr));
                assert_eq!(at.hour(), 18);
            }
        }
        // The 00Z run's first day is Nov 18 06Z to Nov 19 06Z: records f06..f24.
        assert_eq!(window_records(18), [12, 18, 24, 30]);
        assert_eq!(
            frame_date(init(0), 18),
            NaiveDate::from_ymd_opt(2026, 11, 18).unwrap()
        );
        assert_eq!(
            frame_date(init(12), 30),
            NaiveDate::from_ymd_opt(2026, 11, 19).unwrap()
        );
    }

    #[test]
    fn idx_terms_pin_the_six_hour_maximum() {
        assert_eq!(
            tmax_terms(30),
            ["TMAX:2 m above ground", ":24-30 hour max fcst:"]
        );
    }

    #[test]
    fn daily_high_is_the_max_of_the_four_six_hour_maxima() {
        let g = |vals: [f32; 2]| Grid {
            width: 2,
            height: 1,
            values: vals.to_vec(),
            geo: GridGeo::Regular {
                lats: vec![40.0],
                lons: vec![-90.0, -89.75],
            },
        };
        let high = daily_high(&[
            g([280.0, 290.0]),
            g([285.0, f32::NAN]),
            g([283.0, 291.0]),
            g([281.0, 289.0]),
        ])
        .unwrap();
        assert_eq!(high.values[0], 285.0);
        assert!(high.values[1].is_nan());
        assert!((k_to_f(273.15) - 32.0).abs() < 1e-4);
    }
}
