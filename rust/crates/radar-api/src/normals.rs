//! `GET /api/normals/{lat}/{lon}?date=YYYY-MM-DD`: the 1991-2020 normal high and low at one spot
//! (design `weather-extras.md` section 6, "Temp vs normal").
//!
//! Read from the store under `STATE_DIR/static/normals/` (two 2-byte reads per array). `date` is the
//! client's local date and defaults to today in UTC. Outside the lower 48 the answer is `404`; before
//! the store is built it is `503` with `no-store`.

use axum::extract::{Path as UrlPath, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{NaiveDate, Utc};
use radar_aux::static_data::normals::{DayNormal, Normals};
use radar_core::time::round_digits;
use serde_json::{Value, json};

use crate::params::{Issues, query_param};
use crate::respond;
use crate::state::AppState;

/// Cache lifetime: normals never change, the date in the URL keys the answer.
pub const TTL_S: u64 = 86_400;
/// Attribution shown with the line.
pub const SOURCE: &str = "NOAA NCEI nClimGrid-Daily 1991-2020 normals";

/// The JSON answer for one cell and date.
pub fn body(lat: f64, lon: f64, date: NaiveDate, n: DayNormal) -> Value {
    let f = |c: f64| round_digits(c * 1.8 + 32.0, 1);
    json!({
        "lat": lat,
        "lon": lon,
        "date": date.format("%Y-%m-%d").to_string(),
        "high_f": f(n.high_c),
        "low_f": f(n.low_c),
        "high_c": round_digits(n.high_c, 1),
        "low_c": round_digits(n.low_c, 1),
        "period": "1991-2020",
        "source": SOURCE,
    })
}

/// `GET /api/normals/{lat}/{lon}`.
pub async fn get_normals(
    State(state): State<AppState>,
    UrlPath((lat_raw, lon_raw)): UrlPath<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    let mut issues = Issues::new();
    let lat = issues.float_in("path", "lat", Some(&lat_raw), -90.0, 90.0);
    let lon = issues.float_in("path", "lon", Some(&lon_raw), -180.0, 180.0);
    let date_raw = query_param(query.as_deref(), "date");
    let date = match date_raw.as_deref() {
        None => Some(Utc::now().date_naive()),
        Some(raw) => {
            let parsed = NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok();
            if parsed.is_none() {
                issues.push(respond::issue(
                    "date_from_datetime_parsing",
                    &["query", "date"],
                    "Input should be a valid date in the format YYYY-MM-DD",
                    json!(raw),
                ));
            }
            parsed
        }
    };
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon), Some(date)) = (lat, lon, date) else {
        return respond::internal_error();
    };
    let state_dir = state.config.state_dir.clone();
    let got = tokio::task::spawn_blocking(move || -> std::io::Result<Option<Option<DayNormal>>> {
        let Some(n) = Normals::open(&state_dir)? else {
            return Ok(None);
        };
        Ok(Some(n.point(lat, lon, date)?))
    })
    .await;
    match got {
        Ok(Ok(Some(Some(n)))) => respond::json(
            StatusCode::OK,
            &body(lat, lon, date, n),
            Some(&respond::public_max_age(TTL_S)),
        ),
        Ok(Ok(Some(None))) => respond::error_code(StatusCode::NOT_FOUND, "no_normal_here", false),
        Ok(Ok(None)) => {
            respond::error_code(StatusCode::SERVICE_UNAVAILABLE, "normals_not_ready", true)
        }
        Ok(Err(err)) => {
            tracing::warn!(err = %err, "normals_read_failed");
            respond::internal_error()
        }
        Err(_) => respond::internal_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_converts_to_fahrenheit_and_rounds() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let v = body(
            42.2,
            -83.3,
            d,
            DayNormal {
                high_c: 8.3,
                low_c: -0.6,
            },
        );
        assert_eq!(v["date"], "2026-11-18");
        assert_eq!(v["high_f"], 46.9);
        assert_eq!(v["low_f"], 30.9);
        assert_eq!(v["high_c"], 8.3);
    }

    /// The contract fixture the web and app tests read. `NORMALS_FIXTURE_WRITE=1` rewrites it.
    #[test]
    fn contract_fixture_matches_the_server() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../web/src/shared/testdata/normals.contract.json");
        let d = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let fixture = json!({
            "detroit": body(42.33, -83.05, d, DayNormal { high_c: 8.3, low_c: -0.6 }),
            "phoenix_july": body(
                33.43,
                -112.0,
                NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                DayNormal { high_c: 41.5, low_c: 27.2 },
            ),
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
        if std::env::var_os("NORMALS_FIXTURE_WRITE").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk, fixture, "rerun with NORMALS_FIXTURE_WRITE=1");
    }
}
