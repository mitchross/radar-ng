//! `GET /api/sky/tonight/{lat}/{lon}`: visible ISS passes for the next three nights (design section 7).
//!
//! Reads `STATE_DIR/iss.json` (written by `IngestSkyWorkflow`) and runs SGP4 here. The raw elements are
//! never re-served. Elements older than 14 days return no passes and `status: "stale"`.
//!
//! When the `stargazing` layer is published, `stargazing` carries its point scores for the coming
//! night and the best window (the top hour and its neighbours within ten points, at least Good).
//! Hours with stored ECCC fields also carry `seeing` and `transparency` (0-5) and `seeing_source`.

use std::path::Path;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use radar_aux::sky::iss::read_iss;
use radar_aux::sky::passes::{self, Pass, Viewer};
use radar_core::time::parse_iso_as_utc;
use serde_json::{Value, json};

use crate::grid::sample_point;
use crate::params::Issues;
use crate::respond;
use crate::state::AppState;

/// Hours of passes returned: the next three nights.
pub const WINDOW_H: i64 = 72;
/// Passes move slowly; ten minutes of edge caching is safe.
const MAX_AGE_S: u64 = 600;
const CREDIT: &str = "Orbital elements: CelesTrak";
/// Map layer scored 0-100 per night hour.
pub const STARGAZING_LAYER: &str = "stargazing";
/// Hours ahead scanned for tonight's scores.
pub const STARGAZING_AHEAD_H: i64 = 18;
/// A best window needs a top hour at least this good ("Good" on the legend).
pub const BEST_MIN_SCORE: f64 = 50.0;
/// Neighbouring hours within this many points of the top hour join the window.
pub const BEST_SPAN_POINTS: f64 = 10.0;

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
    let grid_dir = state.config.grid_dir.clone();
    let layers = crate::manifest::cached_manifest(&state)
        .await
        .ok()
        .and_then(|m| m.value.get("layers").cloned());
    let body = tokio::task::spawn_blocking(move || {
        let now = Utc::now();
        let mut body = tonight(&dir, lat, lon, now);
        let frames = stargazing_frames(layers.as_ref());
        if !frames.is_empty() {
            let sample = |ts: &str| {
                sample_point(&grid_dir, STARGAZING_LAYER, ts, lat, lon, None)
                    .get("value")
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite())
            };
            body["stargazing"] = stargazing_tonight(&frames, now, sample);
            add_seeing(&mut body["stargazing"], &dir, lat, lon);
        }
        body
    })
    .await;
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

/// `(timestamp, valid time)` of each published stargazing hour, oldest first.
#[must_use]
pub fn stargazing_frames(layers: Option<&Value>) -> Vec<(String, DateTime<Utc>)> {
    let Some(entry) = layers.and_then(|l| l.get(STARGAZING_LAYER)) else {
        return Vec::new();
    };
    let from_frames = entry.get("frames").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(|f| f.get("timestamp").and_then(Value::as_str))
            .collect::<Vec<_>>()
    });
    let stamps = from_frames.unwrap_or_else(|| {
        entry
            .get("timestamps")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    });
    let mut out: Vec<(String, DateTime<Utc>)> = stamps
        .into_iter()
        .filter_map(|s| parse_iso_as_utc(s).map(|t| (s.to_owned(), t)))
        .collect();
    out.sort_by_key(|(_, t)| *t);
    out
}

/// Scores for the hours from now to [`STARGAZING_AHEAD_H`] ahead and the best window in them.
/// `sample` reads the score at a frame; daytime hours have none and break a window.
pub fn stargazing_tonight(
    frames: &[(String, DateTime<Utc>)],
    now: DateTime<Utc>,
    sample: impl Fn(&str) -> Option<f64>,
) -> Value {
    let t = |d: DateTime<Utc>| d.to_rfc3339_opts(SecondsFormat::Secs, true);
    // The hour that is under way counts: a frame valid at 22:00 covers 22:00-23:00.
    let from = now - TimeDelta::hours(1);
    let to = now + TimeDelta::hours(STARGAZING_AHEAD_H);
    let hours: Vec<(DateTime<Utc>, Option<f64>)> = frames
        .iter()
        .filter(|(_, at)| *at > from && *at <= to)
        .map(|(ts, at)| (*at, sample(ts).map(|v| v.clamp(0.0, 100.0).round())))
        .collect();
    let best = best_window(&hours).map(|(a, b, score)| {
        json!({ "start": t(hours[a].0), "end": t(hours[b].0 + TimeDelta::hours(1)), "score": score })
    });
    let list: Vec<Value> = hours
        .iter()
        .map(|(at, v)| json!({ "at": t(*at), "score": v }))
        .collect();
    json!({ "layer": STARGAZING_LAYER, "hours": list, "best": best })
}

/// Adds ECCC `seeing` and `transparency` (0-5) to each hour the aux worker has stored, and the
/// credit when any hour has them. Hours without a file are left as they are.
pub fn add_seeing(stargazing: &mut Value, state_dir: &Path, lat: f64, lon: f64) {
    let Some(hours) = stargazing.get_mut("hours").and_then(Value::as_array_mut) else {
        return;
    };
    let mut any = false;
    for hour in hours.iter_mut() {
        let at = hour
            .get("at")
            .and_then(Value::as_str)
            .and_then(parse_iso_as_utc);
        if let Some((seeing, transparency)) =
            at.and_then(|at| radar_aux::sky::eccc::read_point(state_dir, at, lat, lon))
        {
            hour["seeing"] = json!(seeing);
            hour["transparency"] = json!(transparency);
            any = true;
        }
    }
    if any {
        stargazing["seeing_source"] = json!(radar_aux::sky::eccc::CREDIT);
    }
}

/// `(first, last, top score)` indexes of the best run of consecutive hours, or `None` when no hour
/// reaches [`BEST_MIN_SCORE`]. Ties keep the earliest top hour.
fn best_window(hours: &[(DateTime<Utc>, Option<f64>)]) -> Option<(usize, usize, f64)> {
    let (top, score) = hours
        .iter()
        .enumerate()
        .filter_map(|(i, (_, v))| v.map(|v| (i, v)))
        .fold(None, |acc: Option<(usize, f64)>, (i, v)| match acc {
            Some((_, b)) if b >= v => acc,
            _ => Some((i, v)),
        })?;
    if score < BEST_MIN_SCORE {
        return None;
    }
    let keep = |i: usize| {
        hours[i]
            .1
            .is_some_and(|v| v >= score - BEST_SPAN_POINTS && v >= BEST_MIN_SCORE)
    };
    let next_to = |a: usize, b: usize| hours[b].0 - hours[a].0 == TimeDelta::hours(1);
    let (mut first, mut last) = (top, top);
    while first > 0 && keep(first - 1) && next_to(first - 1, first) {
        first -= 1;
    }
    while last + 1 < hours.len() && keep(last + 1) && next_to(last, last + 1) {
        last += 1;
    }
    Some((first, last, score))
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

    fn night_frames() -> Vec<(String, DateTime<Utc>)> {
        // 2026-11-18 00:00 to 12:00 UTC, every hour.
        (0..=12)
            .map(|h| {
                let at = Utc.with_ymd_and_hms(2026, 11, 18, h, 0, 0).unwrap();
                (at.to_rfc3339_opts(SecondsFormat::Secs, true), at)
            })
            .collect()
    }

    fn score_at(ts: &str) -> Option<f64> {
        // Day until 23:00 local (03 UTC), clear 03-07 UTC with the top at 05, cloudy after.
        let h: u32 = ts[11..13].parse().unwrap();
        match h {
            0..=2 => None,
            3 => Some(60.0),
            4 => Some(78.0),
            5 => Some(84.4),
            6 => Some(80.0),
            7 => Some(73.0),
            8 => Some(76.0),
            _ => Some(20.0),
        }
    }

    #[test]
    fn best_window_is_the_top_hour_and_close_neighbours() {
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 1, 30, 0).unwrap();
        let out = stargazing_tonight(&night_frames(), now, score_at);
        assert_eq!(out["layer"], "stargazing");
        // 01:00 (under way) through 12:00.
        assert_eq!(out["hours"].as_array().unwrap().len(), 12);
        assert_eq!(out["hours"][0]["score"], Value::Null);
        // 04-06 UTC are within 10 points of 84; 07 at 73 is not, so 08 is cut off too.
        assert_eq!(out["best"]["start"], "2026-11-18T04:00:00Z");
        assert_eq!(out["best"]["end"], "2026-11-18T07:00:00Z");
        assert_eq!(out["best"]["score"], 84.0);
    }

    #[test]
    fn no_best_window_below_good_or_without_scores() {
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 1, 0, 0).unwrap();
        let poor = stargazing_tonight(&night_frames(), now, |_| Some(35.0));
        assert_eq!(poor["best"], Value::Null);
        let day = stargazing_tonight(&night_frames(), now, |_| None);
        assert_eq!(day["best"], Value::Null);
        let none = stargazing_tonight(&[], now, score_at);
        assert_eq!(none["hours"], json!([]));
    }

    #[test]
    fn a_gap_in_published_hours_breaks_the_window() {
        let mut frames = night_frames();
        frames.retain(|(ts, _)| !ts.contains("T06:"));
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 3, 0, 0).unwrap();
        let out = stargazing_tonight(&frames, now, score_at);
        assert_eq!(out["best"]["start"], "2026-11-18T04:00:00Z");
        assert_eq!(out["best"]["end"], "2026-11-18T06:00:00Z");
    }

    #[test]
    fn frames_come_from_either_manifest_shape() {
        let frames = json!({ "stargazing": { "frames": [
            { "timestamp": "2026-11-18T05:00:00Z" }, { "timestamp": "2026-11-18T04:00:00Z" }
        ] } });
        let got = stargazing_frames(Some(&frames));
        assert_eq!(got[0].0, "2026-11-18T04:00:00Z");
        let stamps = json!({ "stargazing": { "timestamps": ["2026-11-18T04:00:00Z"] } });
        assert_eq!(stargazing_frames(Some(&stamps)).len(), 1);
        assert!(stargazing_frames(Some(&json!({}))).is_empty());
        assert!(stargazing_frames(None).is_empty());
    }

    #[test]
    fn stored_seeing_joins_its_hours_with_the_credit() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = |n: &str| {
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../radar-grib/testdata")
                    .join(n),
            )
            .unwrap()
        };
        let s =
            radar_grib::rotated::decode_rotated(&fixture("eccc_rdps_seeing_f006_greatlakes.grib2"))
                .unwrap();
        let t = radar_grib::rotated::decode_rotated(&fixture(
            "eccc_rdps_transparency_f006_greatlakes.grib2",
        ))
        .unwrap();
        let hour = radar_grib::seeing::SeeingHour::from_fields(&s, &t, 0).unwrap();
        let sub = dir.path().join(radar_grib::seeing::SEEING_DIR);
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("20261118T06Z.bin"), hour.encode()).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 3, 0, 0).unwrap();
        let mut out = stargazing_tonight(&night_frames(), now, score_at);
        add_seeing(&mut out, dir.path(), 43.65, -79.38);
        let six = out["hours"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["at"] == "2026-11-18T06:00:00Z")
            .unwrap();
        assert_eq!(
            (six["seeing"].clone(), six["transparency"].clone()),
            (json!(3), json!(3))
        );
        assert!(out["hours"][0].get("seeing").is_none());
        assert_eq!(
            out["seeing_source"],
            "Data Source: Environment and Climate Change Canada"
        );
        // Outside the stored grid nothing is added.
        let mut miami = stargazing_tonight(&night_frames(), now, score_at);
        add_seeing(&mut miami, dir.path(), 25.76, -80.19);
        assert!(miami.get("seeing_source").is_none());
    }
}
