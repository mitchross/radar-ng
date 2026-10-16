//! Pure global-model rules: the hours a run renders, the CONUS crop, the publishable prefix and the
//! manifest frames.

use chrono::DateTime;
use chrono::Utc;
use radar_core::manifest::FrameSpec;
use radar_geo::{Grid, GridGeo};
use radar_types::payloads::hrrr::ForecastHourResult;

use super::{MAX_ZOOM, Product, SPATIAL_RESOLUTION_KM};
use crate::config::IngestConfig;

/// A lat/lon box, degrees, longitudes in [-180, 180].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bbox {
    /// Southern edge.
    pub south: f64,
    /// Northern edge.
    pub north: f64,
    /// Western edge.
    pub west: f64,
    /// Eastern edge.
    pub east: f64,
}

/// The area rendered: the HRRR domain plus a margin (southern Canada, northern Mexico, the coasts).
pub const CONUS_BBOX: Bbox = Bbox {
    south: 18.0,
    north: 58.0,
    west: -140.0,
    east: -50.0,
};

/// Forecast hours a run renders, ascending.
pub fn forecast_hours(product: Product, cfg: &IngestConfig) -> Vec<u32> {
    match product {
        Product::GfsRadar => (cfg.gfs_radar_first_hour..=cfg.gfs_radar_last_hour).collect(),
        Product::Uv => (1..=cfg.uv_last_hour).collect(),
    }
}

/// Whether the product is switched on.
pub fn enabled(product: Product, cfg: &IngestConfig) -> bool {
    match product {
        Product::GfsRadar => cfg.gfs_radar_enabled,
        Product::Uv => cfg.uv_enabled,
    }
}

/// The cells of a regular grid inside `bbox`, south-up with ascending longitudes.
///
/// Global GRIB grids run 0..360 east, which the decoder folds to [-180, 180]; the longitude axis is then
/// not monotonic, so rows and columns are picked and sorted by coordinate rather than sliced.
pub fn crop(grid: &Grid, bbox: Bbox) -> anyhow::Result<Grid> {
    let GridGeo::Regular { lats, lons } = &grid.geo else {
        anyhow::bail!("global-model fields must be on a regular lat/lon grid");
    };
    let pick = |axis: &[f64], lo: f64, hi: f64| {
        let mut idx: Vec<usize> = (0..axis.len())
            .filter(|&i| axis[i] >= lo && axis[i] <= hi)
            .collect();
        idx.sort_by(|&a, &b| axis[a].total_cmp(&axis[b]));
        idx
    };
    let rows = pick(lats, bbox.south, bbox.north);
    let cols = pick(lons, bbox.west, bbox.east);
    if rows.len() < 2 || cols.len() < 2 {
        anyhow::bail!("grid does not cover the crop box");
    }
    let mut values = Vec::with_capacity(rows.len() * cols.len());
    for &r in &rows {
        values.extend(cols.iter().map(|&c| grid.values[r * grid.width + c]));
    }
    Ok(Grid {
        width: cols.len(),
        height: rows.len(),
        values,
        geo: GridGeo::Regular {
            lats: rows.iter().map(|&r| lats[r]).collect(),
            lons: cols.iter().map(|&c| lons[c]).collect(),
        },
    })
}

/// Results in hour order, from the first planned hour, while each hour is the next planned one and
/// rendered the layer. The first gap ends the prefix.
pub fn publishable_prefix<'a>(
    results: &'a [ForecastHourResult],
    hours: &[u32],
    layer: &str,
) -> Vec<&'a ForecastHourResult> {
    let mut sorted: Vec<&ForecastHourResult> = results.iter().collect();
    sorted.sort_by_key(|r| r.fhr);
    sorted
        .into_iter()
        .zip(hours)
        .take_while(|(r, h)| {
            r.fhr == i64::from(**h) && r.rendered_layers.iter().any(|l| l == layer)
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
            extra.insert("kind".into(), "model_guidance".into());
            extra.insert("issued_at".into(), issued_at.clone().into());
            extra.insert("run_id".into(), run_id.into());
            extra.insert("lead_minutes".into(), serde_json::json!(r.fhr * 60));
            extra.insert(
                "spatial_resolution_km".into(),
                serde_json::json!(SPATIAL_RESOLUTION_KM),
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
    use radar_core::env::Paths;

    fn hour(fhr: i64, layers: &[&str]) -> ForecastHourResult {
        ForecastHourResult {
            fhr,
            rendered_layers: layers.iter().map(|s| s.to_string()).collect(),
            duration_s: 0.0,
            valid_timestamp: String::new(),
        }
    }

    /// A 2.5 degree global grid as stored in GFS files: north first, longitudes 0..357.5 folded.
    fn global_grid() -> Grid {
        let lats: Vec<f64> = (0..73).map(|i| 90.0 - 2.5 * i as f64).collect();
        let lons: Vec<f64> = (0..144)
            .map(|j| {
                let l = 2.5 * j as f64;
                if l > 180.0 { l - 360.0 } else { l }
            })
            .collect();
        let values = lats
            .iter()
            .flat_map(|&la| lons.iter().map(move |&lo| (la * 1000.0 + lo) as f32))
            .collect();
        Grid {
            width: 144,
            height: 73,
            values,
            geo: GridGeo::Regular { lats, lons },
        }
    }

    #[test]
    fn crop_is_south_up_with_ascending_longitudes_across_the_fold() {
        let g = global_grid();
        let c = crop(&g, CONUS_BBOX).unwrap();
        let GridGeo::Regular { lats, lons } = &c.geo else {
            panic!("regular")
        };
        assert_eq!(lats.first(), Some(&20.0));
        assert_eq!(lats.last(), Some(&57.5));
        assert_eq!(lons.first(), Some(&-140.0));
        assert_eq!(lons.last(), Some(&-50.0));
        assert!(lats.windows(2).all(|w| w[0] < w[1]));
        assert!(lons.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(c.values.len(), lats.len() * lons.len());
        // Each value still sits at its own coordinate.
        assert_eq!(c.at(0, 0), (20.0 * 1000.0 - 140.0) as f32);
        assert_eq!(
            c.at(c.height - 1, c.width - 1),
            (57.5 * 1000.0 - 50.0) as f32
        );

        // A box across the antimeridian's opposite side (0 degrees) is also contiguous after sorting.
        let europe = crop(
            &g,
            Bbox {
                south: 40.0,
                north: 50.0,
                west: -10.0,
                east: 10.0,
            },
        )
        .unwrap();
        let GridGeo::Regular { lons, .. } = &europe.geo else {
            panic!("regular")
        };
        assert_eq!(lons.first(), Some(&-10.0));
        assert_eq!(lons.last(), Some(&10.0));
        assert!(lons.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn crop_outside_the_grid_fails() {
        let g = global_grid();
        let tiny = Bbox {
            south: 1.0,
            north: 2.0,
            west: 1.0,
            east: 2.0,
        };
        assert!(crop(&g, tiny).is_err());
    }

    #[test]
    fn prefix_starts_at_the_first_planned_hour_and_stops_at_a_gap() {
        let hours = [24, 25, 26, 27];
        let results = vec![
            hour(26, &["radar-gfs"]),
            hour(24, &["radar-gfs"]),
            hour(25, &["radar-gfs"]),
            hour(27, &[]),
        ];
        let p = publishable_prefix(&results, &hours, "radar-gfs");
        assert_eq!(p.iter().map(|r| r.fhr).collect::<Vec<_>>(), [24, 25, 26]);
        // Missing first hour: nothing is publishable.
        let late = vec![hour(25, &["radar-gfs"])];
        assert!(publishable_prefix(&late, &hours, "radar-gfs").is_empty());
        // Another layer does not count.
        let other = vec![hour(24, &["uv"])];
        assert!(publishable_prefix(&other, &hours, "radar-gfs").is_empty());
    }

    #[test]
    fn frames_carry_run_path_and_lead() {
        let init = radar_grib::hrrr::parse_run_id("20261015_12").unwrap();
        let results = [hour(24, &["radar-gfs"]), hour(25, &["radar-gfs"])];
        let refs: Vec<&ForecastHourResult> = results.iter().collect();
        let f = frames(Product::GfsRadar, "20261015_12", init, &refs);
        assert_eq!(f[0].timestamp, "2026-10-16T12:00:00+00:00");
        assert_eq!(
            f[0].path.as_deref(),
            Some("runs/20261015_12/2026-10-16T12:00:00+00:00")
        );
        assert_eq!(f[1].extra["lead_minutes"], 25 * 60);
        assert_eq!(f[1].extra["source"], "gfs");
        assert_eq!(f[1].extra["issued_at"], "2026-10-15T12:00:00+00:00");
    }

    #[test]
    fn hours_follow_config_and_products_round_trip() {
        let p = std::path::PathBuf::from;
        let paths = Paths::new(p("/s"), p("/g"), p("/t"));
        let mut cfg = IngestConfig::with_roots(paths, p("/w")).unwrap();
        cfg.gfs_radar_first_hour = 30;
        cfg.gfs_radar_last_hour = 33;
        cfg.uv_last_hour = 3;
        assert_eq!(forecast_hours(Product::GfsRadar, &cfg), [30, 31, 32, 33]);
        assert_eq!(forecast_hours(Product::Uv, &cfg), [1, 2, 3]);
        for p in Product::ALL {
            assert_eq!(Product::from_layer(p.layer()).unwrap(), p);
        }
        assert!(Product::from_layer("radar-hrrr").is_err());
        assert_eq!(Product::Uv.convert(0.25), 10.0);
        assert_eq!(Product::GfsRadar.convert(42.5), 42.5);
    }
}
