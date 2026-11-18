//! `GET /api/sky/tonight/{lat}/{lon}`: visible ISS passes for the next three nights (design section 7).
//!
//! Reads `STATE_DIR/iss.json` (written by `IngestSkyWorkflow`) and runs SGP4 here. The raw elements are
//! never re-served. Elements older than 14 days return no passes and `status: "stale"`.

use std::path::Path;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, SecondsFormat, Utc};
use radar_aux::sky::iss::read_iss;
use radar_aux::sky::passes::{self, Pass, Viewer};
use serde_json::{Value, json};

use crate::params::Issues;
use crate::respond;
use crate::state::AppState;

/// Hours of passes returned: the next three nights.
pub const WINDOW_H: i64 = 72;
/// Passes move slowly; ten minutes of edge caching is safe.
const MAX_AGE_S: u64 = 600;
const CREDIT: &str = "Orbital elements: CelesTrak";

/// `GET /api/sky/tonight/{lat}/{lon}`.
pub async fn get_sky_tonight(
    State(state): State<AppState>,
    UrlPath((lat_raw, lon_raw)): UrlPath<(String, String)>,
) -> Response {
    let mut issues = Issues::new();
    let lat = issues.float_in("path", "lat", Some(&lat_raw), -90.0, 90.0);
    let lon = issues.float_in("path", "lon", Some(&lon_raw), -180.0, 180.0);
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return respond::internal_error();
    };
    let dir = state.config.state_dir.clone();
    let body = tokio::task::spawn_blocking(move || tonight(&dir, lat, lon, Utc::now())).await;
    match body {
        Ok(body) => {
            let cc = if body["status"] == "ok" {
                respond::public_max_age(MAX_AGE_S)
            } else {
                "no-store".to_owned()
            };
            respond::json(StatusCode::OK, &body, Some(&cc))
        }
        Err(_) => respond::internal_error(),
    }
}

/// The response body for one viewer at `now`.
pub fn tonight(state_dir: &Path, lat: f64, lon: f64, now: DateTime<Utc>) -> Value {
    let Some(file) = read_iss(state_dir) else {
        return json!({ "status": "no_data", "reason": "no_elements", "source": CREDIT, "passes": [] });
    };
    let epoch = file.elements.datetime.and_utc();
    let epoch_s = epoch.to_rfc3339_opts(SecondsFormat::Secs, true);
    if passes::elements_stale(epoch, now) {
        return json!({
            "status": "stale",
            "reason": "elements_older_than_14_days",
            "elements_epoch": epoch_s,
            "source": CREDIT,
            "passes": [],
        });
    }
    let viewer = Viewer {
        lat,
        lon,
        height_km: 0.0,
    };
    let list: Vec<Value> = passes::visible_passes(&file.elements, viewer, now, WINDOW_H)
        .iter()
        .map(pass_json)
        .collect();
    json!({
        "status": "ok",
        "elements_epoch": epoch_s,
        "source": CREDIT,
        "passes": list,
    })
}

fn pass_json(p: &Pass) -> Value {
    let t = |d: DateTime<Utc>| d.to_rfc3339_opts(SecondsFormat::Secs, true);
    json!({
        "start": t(p.start),
        "max_at": t(p.max_at),
        "end": t(p.end),
        "max_elevation_deg": p.max_elevation_deg,
        "start_azimuth_deg": p.start_azimuth_deg,
        "max_azimuth_deg": p.max_azimuth_deg,
        "end_azimuth_deg": p.end_azimuth_deg,
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use radar_aux::sky::iss::{ISS_FILE, IssFile, parse_omm_csv};

    use super::*;

    const FIXTURE: &str = include_str!("../../radar-aux/testdata/sky/iss-25544-2026-11-17.csv");

    fn state_with_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let file = IssFile {
            fetched_at: Utc.with_ymd_and_hms(2026, 11, 17, 18, 0, 0).unwrap(),
            elements: parse_omm_csv(FIXTURE).unwrap(),
        };
        std::fs::write(
            dir.path().join(ISS_FILE),
            serde_json::to_vec(&file).unwrap(),
        )
        .unwrap();
        dir
    }

    #[test]
    fn detroit_tonight_lists_the_published_pass() {
        let dir = state_with_fixture();
        let now = Utc.with_ymd_and_hms(2026, 11, 17, 21, 0, 0).unwrap();
        let body = tonight(dir.path(), 42.3314, -83.0458, now);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["source"], CREDIT);
        let first = &body["passes"][0];
        // Heavens-Above: visible from 23:47:42 to 23:50:53 UTC, highest 21°.
        assert!(
            first["start"]
                .as_str()
                .unwrap()
                .starts_with("2026-11-17T23:4"),
            "{first}"
        );
        assert!(
            body.get("elements").is_none(),
            "raw elements are never re-served"
        );
    }

    #[test]
    fn stale_and_missing_elements_say_why() {
        let dir = state_with_fixture();
        let later = Utc.with_ymd_and_hms(2026, 12, 2, 0, 0, 0).unwrap();
        let body = tonight(dir.path(), 42.3, -83.0, later);
        assert_eq!(body["status"], "stale");
        assert_eq!(body["reason"], "elements_older_than_14_days");
        assert_eq!(body["passes"], json!([]));

        let empty = tempfile::tempdir().unwrap();
        let body = tonight(empty.path(), 42.3, -83.0, later);
        assert_eq!(body["status"], "no_data");
    }
}
