//! Pure winter rules: the hours a run renders, running totals, the publishable prefix and the manifest
//! frames.

use chrono::{DateTime, Utc};
use radar_core::manifest::FrameSpec;
use radar_types::payloads::hrrr::ForecastHourResult;

use super::{MAX_ZOOM, Product};
use crate::config::IngestConfig;

/// Last NBM forecast hour summed into the totals.
pub const NBM_LAST_HOUR: u32 = 72;

/// Forecast hours a run renders, ascending.
pub fn forecast_hours(product: Product) -> Vec<u32> {
    match product {
        Product::NbmSnow => (1..=NBM_LAST_HOUR / radar_grib::nbm::STEP_HOURS)
            .map(|k| k * radar_grib::nbm::STEP_HOURS)
            .collect(),
        // One grid at its valid time.
        Product::SnowDepth | Product::Snowfall24h => vec![0],
        // Listed per cycle by the find step (`wpc::listed_hours`).
        Product::SnowChance => Vec::new(),
    }
}

/// Hours a run planned: the fixed table, or for a listed product the hours of its results (the
/// workflow passes one result per hour the find step returned, failed ones included).
pub fn planned_hours(product: Product, results: &[ForecastHourResult]) -> Vec<u32> {
    if !product.listed_hours() {
        return forecast_hours(product);
    }
    let mut hours: Vec<u32> = results
        .iter()
        .filter_map(|r| u32::try_from(r.fhr).ok())
        .collect();
    hours.sort_unstable();
    hours.dedup();
    hours
}

/// Whether the product is switched on.
pub fn enabled(product: Product, cfg: &IngestConfig) -> bool {
    match product {
        Product::NbmSnow | Product::SnowDepth | Product::Snowfall24h | Product::SnowChance => {
            cfg.winter_enabled
        }
    }
}

/// The 6 hour records that make up the total at `fhr`: ending at 6, 12, ... `fhr`.
pub fn record_hours(fhr: u32) -> Vec<u32> {
    let step = radar_grib::nbm::STEP_HOURS;
    (1..=fhr / step).map(|k| k * step).collect()
}

/// Add `step` into `total` cell by cell. A NaN cell (outside the NBM domain) stays NaN.
pub fn accumulate(total: &mut [f32], step: &[f32]) -> anyhow::Result<()> {
    if total.len() != step.len() {
        anyhow::bail!(
            "record grids differ: {} vs {} cells",
            total.len(),
            step.len()
        );
    }
    for (t, s) in total.iter_mut().zip(step) {
        *t += s.max(0.0);
    }
    Ok(())
}

/// Results in hour order, from the first planned hour, while each hour is the next planned one and
/// rendered every layer of the product. The first gap ends the prefix.
pub fn publishable_prefix<'a>(
    results: &'a [ForecastHourResult],
    hours: &[u32],
    product: Product,
) -> Vec<&'a ForecastHourResult> {
    let mut sorted: Vec<&ForecastHourResult> = results.iter().collect();
    sorted.sort_by_key(|r| r.fhr);
    sorted
        .into_iter()
        .zip(hours)
        .take_while(|(r, h)| {
            r.fhr == i64::from(**h)
                && product
                    .layers()
                    .iter()
                    .all(|l| r.rendered_layers.iter().any(|x| x == l.id))
        })
        .map(|(r, _)| r)
        .collect()
}

/// Manifest frames for the prefix: `runs/<run>/<valid>`, with model provenance.
pub fn frames(
    product: Product,
    run_id: &str,
    init: DateTime<Utc>,
    prefix: &[&ForecastHourResult],
) -> Vec<FrameSpec> {
    let issued_at = radar_grib::mrms::format_valid_time(init);
    prefix
        .iter()
        .map(|r| {
            let valid = crate::hrrr::valid_time(init, r.fhr as u32);
            let mut extra = serde_json::Map::new();
            extra.insert("source".into(), product.source().into());
            extra.insert("kind".into(), product.kind().into());
            extra.insert("issued_at".into(), issued_at.clone().into());
            extra.insert("run_id".into(), run_id.into());
            extra.insert("lead_minutes".into(), serde_json::json!(r.fhr * 60));
            extra.insert(
                "spatial_resolution_km".into(),
                serde_json::json!(product.resolution_km()),
            );
            extra.insert("max_zoom".into(), serde_json::json!(MAX_ZOOM));
            FrameSpec {
                path: Some(format!("runs/{run_id}/{valid}")),
                timestamp: valid,
                palettes: Vec::new(),
                extra,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(fhr: i64, layers: &[&str]) -> ForecastHourResult {
        ForecastHourResult {
            fhr,
            rendered_layers: layers.iter().map(|s| s.to_string()).collect(),
            duration_s: 0.0,
            valid_timestamp: String::new(),
        }
    }

    const BOTH: [&str; 2] = ["snowfall-forecast", "ice-forecast"];

    #[test]
    fn twelve_six_hour_frames_to_72() {
        let h = forecast_hours(Product::NbmSnow);
        assert_eq!(h.len(), 12);
        assert_eq!((h[0], h[11]), (6, 72));
        assert_eq!(record_hours(18), [6, 12, 18]);
        assert!(record_hours(5).is_empty());
    }

    #[test]
    fn totals_add_and_keep_nan() {
        let mut t = vec![0.0, 1.0, f32::NAN];
        accumulate(&mut t, &[0.5, -0.1, 1.0]).unwrap();
        assert_eq!(&t[..2], &[0.5, 1.0]);
        assert!(t[2].is_nan());
        assert!(accumulate(&mut t, &[1.0]).is_err());
    }

    #[test]
    fn prefix_needs_every_layer() {
        let hours = forecast_hours(Product::NbmSnow);
        let results = vec![hour(12, &BOTH), hour(6, &BOTH), hour(18, &BOTH[..1])];
        let p = publishable_prefix(&results, &hours, Product::NbmSnow);
        assert_eq!(p.iter().map(|r| r.fhr).collect::<Vec<_>>(), [6, 12]);
        assert!(publishable_prefix(&[hour(12, &BOTH)], &hours, Product::NbmSnow).is_empty());
    }

    #[test]
    fn frames_carry_run_path_and_lead() {
        let init = radar_grib::hrrr::parse_run_id("20261117_12").unwrap();
        let results = [hour(6, &BOTH)];
        let refs: Vec<&ForecastHourResult> = results.iter().collect();
        let f = frames(Product::NbmSnow, "20261117_12", init, &refs);
        assert_eq!(f[0].timestamp, "2026-11-17T18:00:00+00:00");
        assert_eq!(
            f[0].path.as_deref(),
            Some("runs/20261117_12/2026-11-17T18:00:00+00:00")
        );
        assert_eq!(f[0].extra["lead_minutes"], 360);
        assert_eq!(f[0].extra["source"], "nbm");
    }

    #[test]
    fn snow_chance_plans_the_listed_hours_and_frames_end_the_window() {
        let three = ["snow-chance-1", "snow-chance-4", "snow-chance-8"];
        assert!(forecast_hours(Product::SnowChance).is_empty());
        let results = vec![hour(29, &three), hour(23, &three), hour(35, &three[..2])];
        let hours = planned_hours(Product::SnowChance, &results);
        assert_eq!(hours, [23, 29, 35]);
        let p = publishable_prefix(&results, &hours, Product::SnowChance);
        assert_eq!(p.iter().map(|r| r.fhr).collect::<Vec<_>>(), [23, 29]);
        assert_eq!(planned_hours(Product::NbmSnow, &[]).len(), 12);

        let init = radar_grib::hrrr::parse_run_id("20261118_01").unwrap();
        let f = frames(Product::SnowChance, "20261118_01", init, &p);
        // f023 of the 01Z cycle is the 24 h ending 00Z the next day.
        assert_eq!(f[0].timestamp, "2026-11-19T00:00:00+00:00");
        assert_eq!(f[1].extra["lead_minutes"], 29 * 60);
        assert_eq!(f[0].extra["source"], "wpc");
        assert_eq!(f[0].extra["kind"], "model_guidance");
    }

    #[test]
    fn observed_products_are_one_frame_at_their_valid_time() {
        for p in [Product::SnowDepth, Product::Snowfall24h] {
            assert_eq!(forecast_hours(p), [0]);
        }
        let init = radar_grib::hrrr::parse_run_id("20261117_06").unwrap();
        let results = [hour(0, &["snow-depth"])];
        let refs: Vec<&ForecastHourResult> = results.iter().collect();
        assert_eq!(
            publishable_prefix(&results, &[0], Product::SnowDepth).len(),
            1
        );
        let f = frames(Product::SnowDepth, "20261117_06", init, &refs);
        assert_eq!(f[0].timestamp, "2026-11-17T06:00:00+00:00");
        assert_eq!(f[0].extra["kind"], "observation");
        assert_eq!(f[0].extra["spatial_resolution_km"], 1.0);
        assert_eq!(f[0].extra["lead_minutes"], 0);
    }
}
