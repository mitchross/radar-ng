//! `GET /api/briefing` and `GET /api/alerts/explain`: optional LLM narration (spec 7, 7.5).
//!
//! Decoration only. Every failure answers `200 {"available":false,"reason":...}` with `no-store`,
//! so an LLM outage never breaks radar, forecast or alerts. The LLM is any OpenAI-compatible
//! `POST {LLM_BASE_URL}/chat/completions` server.
//!
//! Briefing single-flight: the first request for a key becomes the leader and publishes its result
//! on a `watch` channel. Followers wait on that channel. The leader drops its sender when done, so a
//! follower sees `None` (unavailable) if the leader failed or was cancelled.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDateTime, Utc};
use radar_core::pycompat::{Style, dumps_value, float_repr, is_falsy, py_str};
use radar_core::time::{now_isoformat, round_digits, round_half_even};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::alerts::alerts_for;
use crate::forecast::forecast_for;
use crate::nowcast::nowcast_point;
use crate::params::{Issues, query_param};
use crate::respond;
use crate::state::AppState;
use crate::upstream;

/// System prompt for the briefing (spec 7.5, verbatim).
pub const BRIEFING_SYSTEM: &str = r#"You narrate the weather for a personal radar app. Be warm, a little playful, and concise. Use ONLY the JSON facts given; never invent numbers, alerts or storms. If there is an active alert or an incoming storm, say that first and plainly. Use local clock times like 2:40 PM. Reply as JSON: {"headline": "<= 8 words", "body": "1-2 short sentences, <= 45 words"}."#;

/// System prompt for the alert explainer (spec 7.5, verbatim).
pub const EXPLAIN_SYSTEM: &str = r#"You explain official NWS weather alerts to a regular person. Use ONLY the alert text given. Reply as JSON: {"what": "one plain sentence on what is happening and where", "do": "one plain sentence on what to do"}. No hype, no emojis."#;

/// WMO weather codes as labels (`WMO` in `routes_briefing.py`).
const WMO: &[(i64, &str)] = &[
    (0, "clear"),
    (1, "mostly clear"),
    (2, "partly cloudy"),
    (3, "overcast"),
    (45, "fog"),
    (48, "freezing fog"),
    (51, "light drizzle"),
    (53, "drizzle"),
    (55, "heavy drizzle"),
    (61, "light rain"),
    (63, "rain"),
    (65, "heavy rain"),
    (66, "freezing rain"),
    (67, "heavy freezing rain"),
    (71, "light snow"),
    (73, "snow"),
    (75, "heavy snow"),
    (77, "snow grains"),
    (80, "rain showers"),
    (81, "heavy showers"),
    (82, "violent showers"),
    (85, "snow showers"),
    (86, "heavy snow showers"),
    (95, "thunderstorms"),
    (96, "thunderstorms with hail"),
    (99, "severe thunderstorms with hail"),
];

/// Earth radius used by the briefing's haversine (`6371.0`, not the storm module's `6371.0088`).
const EARTH_RADIUS_KM: f64 = 6371.0;

/// The incoming-storm search radius in kilometres.
const STORM_RADIUS_KM: f64 = 12.0;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn unavailable(reason: &str) -> Response {
    respond::json(
        StatusCode::OK,
        &json!({ "available": false, "reason": reason }),
        Some("no-store"),
    )
}

fn ok_json(body: &Value, max_age: u64) -> Response {
    respond::json(
        StatusCode::OK,
        body,
        Some(&respond::public_max_age(max_age)),
    )
    .into_response()
}

/// `value` when it is present and truthy (Python `d.get(k)` followed by `or`).
fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| !is_falsy(v))
}

/// Python `_clean(text, limit)`: `str(text or "")`, whitespace collapsed and stripped, first `limit` characters.
#[must_use]
pub fn clean(value: Option<&Value>, limit: usize) -> String {
    let text = match truthy(value) {
        Some(v) => py_str(v),
        None => String::new(),
    };
    clean_text(&text, limit)
}

/// `_clean` for a string already in hand.
#[must_use]
pub fn clean_text(text: &str, limit: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

/// The WMO label for a weather code. Non-integral or missing codes are `unknown`.
#[must_use]
pub fn wmo_label(code: Option<&Value>) -> &'static str {
    let number = match code {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        _ => None,
    };
    let Some(number) = number else {
        return "unknown";
    };
    WMO.iter()
        .find(|(k, _)| *k == number)
        .map_or("unknown", |(_, label)| label)
}

/// `_r(value)`: Python `round()` for numbers, `null` otherwise.
fn round_value(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.is_finite() => json!(round_half_even(f) as i64),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

/// A numeric field read as Python `float(...)`, with `default` when the key is absent.
fn float_field(obj: &Value, key: &str, default: f64) -> f64 {
    match obj.get(key) {
        None => default,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(default),
        Some(_) => default,
    }
}

/// Parse an Open-Meteo or radar timestamp. Offset-aware values keep their wall clock, and naive
/// values are taken as local, as `datetime.fromisoformat` does.
fn parse_wall(ts: &str) -> Option<(NaiveDateTime, bool)> {
    let normalized = ts.replace('Z', "+00:00");
    if let Ok(dt) = DateTime::parse_from_rfc3339(&normalized) {
        return Some((dt.naive_local(), true));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&normalized, fmt) {
            return Some((naive, false));
        }
    }
    None
}

/// `_local(ts, offset)`: `%-I:%M %p` of the timestamp plus the UTC offset (naive stamps get no shift).
fn local_label(ts: &str, offset_s: i64) -> String {
    match parse_wall(ts) {
        Some((naive, aware)) => {
            let wall = if aware {
                naive + chrono::Duration::seconds(offset_s)
            } else {
                naive
            };
            wall.format("%-I:%M %p").to_string()
        }
        None => String::new(),
    }
}

/// `summarize_nowcast`: "dry" or the rain start, peak and end within the next hour.
#[must_use]
pub fn summarize_nowcast(points: &[Value], offset_s: i64) -> String {
    let dbz_of = |p: &Value| p.get("dbz").and_then(Value::as_f64).unwrap_or(-99.0);
    let wet: Vec<usize> = (0..points.len())
        .filter(|&i| dbz_of(&points[i]) >= 15.0)
        .collect();
    let Some(&first) = wet.first() else {
        return "dry for the next hour".to_string();
    };
    let last = *wet.last().unwrap_or(&first);
    let mut peak = first;
    for &i in &wet {
        if dbz_of(&points[i]) > dbz_of(&points[peak]) {
            peak = i;
        }
    }
    let stamp = |i: usize| {
        points[i]
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("")
    };
    let mut parts = vec![
        format!("rain from about {}", local_label(stamp(first), offset_s)),
        format!(
            "peaking near {} at {} dBZ",
            local_label(stamp(peak), offset_s),
            round_half_even(dbz_of(&points[peak])) as i64
        ),
    ];
    if last + 1 < points.len() {
        parts.push(format!(
            "ending around {}",
            local_label(stamp(last + 1), offset_s)
        ));
    }
    parts.join(", ")
}

/// Great-circle distance in kilometres (`_haversine_km`).
#[must_use]
pub fn haversine_km(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let h = ((p2 - p1) / 2.0).sin().powi(2)
        + p1.cos() * p2.cos() * ((lon2 - lon1).to_radians() / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * h.sqrt().min(1.0).asin()
}

/// `incoming_storm`: the soonest tracked cell whose next-hour path passes within 12 km of the point.
#[must_use]
pub fn incoming_storm(storms: &Value, lat: f64, lon: f64) -> Option<Value> {
    let features = storms.get("features").and_then(Value::as_array)?;
    let mut best: Option<(i64, Value)> = None;
    for feature in features {
        let Some(coords) = feature
            .get("geometry")
            .and_then(|g| g.get("coordinates"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let (Some(clon), Some(clat)) = (
            coords.first().and_then(Value::as_f64),
            coords.get(1).and_then(Value::as_f64),
        ) else {
            continue;
        };
        let Some(props) = feature.get("properties").filter(|p| p.is_object()) else {
            continue;
        };
        let vector = truthy(props.get("tracking_vector"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let history = props
            .get("track_history")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let tracked = history >= 3 && float_field(props, "tracking_confidence", 0.0) >= 0.4;
        for minutes in (0..=60i64).step_by(5) {
            if minutes > 0 && !tracked {
                break;
            }
            let h = minutes as f64 / 60.0;
            let east = float_field(&vector, "east_kmh", 0.0);
            let north = float_field(&vector, "north_kmh", 0.0);
            let plon = clon + east * h / f64::max(1e-6, 111.32 * clat.to_radians().cos());
            let plat = clat + north * h / 110.574;
            if haversine_km(plon, plat, lon, lat) <= STORM_RADIUS_KM {
                let candidate = json!({
                    "minutes": minutes,
                    "peak_dbz": round_half_even(float_field(props, "peak_dbz", 0.0)) as i64,
                    "speed_mph": round_half_even(float_field(&vector, "speed_kmh", 0.0) * 0.621) as i64,
                });
                if best.as_ref().is_none_or(|(m, _)| minutes < *m) {
                    best = Some((minutes, candidate));
                }
                break;
            }
        }
    }
    best.map(|(_, value)| value)
}

/// The facts object sent to the model (`build_facts`), in the Python key order.
#[must_use]
pub fn build_facts(
    place: &str,
    forecast: &Value,
    nowcast: &Value,
    alerts: &[Value],
    storm: Option<Value>,
    now: DateTime<Utc>,
) -> Value {
    let offset = py_int_or_zero(forecast.get("utc_offset_seconds"));
    let cur = truthy(forecast.get("current"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let hourly = truthy(forecast.get("hourly"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let daily = truthy(forecast.get("daily"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let now_local = (now + chrono::Duration::seconds(offset)).naive_utc();

    let times: Vec<String> = hourly
        .get("time")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|t| t.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    let now_hour = now_local.format("%Y-%m-%dT%H:00").to_string();
    let start = times
        .iter()
        .position(|t| t.as_str() >= now_hour.as_str())
        .unwrap_or(0);
    let series = |key: &str, i: usize| -> Value {
        hourly
            .get(key)
            .and_then(Value::as_array)
            .and_then(|a| a.get(i))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let mut next_hours: Vec<Value> = Vec::new();
    for i in (start..(start + 12).min(times.len())).step_by(3) {
        let time = parse_wall(&times[i])
            .map(|(naive, _)| naive.format("%-I %p").to_string())
            .unwrap_or_default();
        next_hours.push(json!({
            "time": time,
            "temp_f": round_value(Some(&series("temperature_2m", i))),
            "sky": wmo_label(Some(&series("weather_code", i))),
            "rain_in": series("precipitation", i),
        }));
    }
    let first = |key: &str| -> Value {
        daily
            .get(key)
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(Value::Null)
    };
    let points = nowcast
        .get("points")
        .and_then(Value::as_array)
        .filter(|p| !p.is_empty());
    let next_hour_radar = match points {
        Some(points) => summarize_nowcast(points, offset),
        None => "radar nowcast unavailable".to_string(),
    };
    let active: Vec<Value> = alerts
        .iter()
        .take(3)
        .map(|a| {
            let until = truthy(a.get("ends"))
                .or_else(|| truthy(a.get("expires")))
                .cloned()
                .unwrap_or(Value::Null);
            json!({ "event": a.get("event").cloned().unwrap_or(Value::Null), "until": until })
        })
        .collect();
    json!({
        "place": if place.is_empty() { "this spot" } else { place },
        "local_time": now_local.format("%A %-I:%M %p").to_string(),
        "now": {
            "temp_f": round_value(cur.get("temperature_2m")),
            "feels_like_f": round_value(cur.get("apparent_temperature")),
            "sky": wmo_label(cur.get("weather_code")),
            "wind_mph": round_value(cur.get("wind_speed_10m")),
            "gusts_mph": round_value(cur.get("wind_gusts_10m")),
        },
        "next_hour_radar": next_hour_radar,
        "incoming_storm": storm,
        "next_12_hours": next_hours,
        "today": {
            "high_f": round_value(Some(&first("temperature_2m_max"))),
            "low_f": round_value(Some(&first("temperature_2m_min"))),
            "rain_total_in": first("precipitation_sum"),
        },
        "active_alerts": active,
    })
}

fn py_int_or_zero(value: Option<&Value>) -> i64 {
    radar_core::pycompat::py_int_or_zero(value).unwrap_or(0)
}

/// `_parse_json`: drop `<think>...</think>` spans, then parse from the first `{` to the last `}`.
#[must_use]
pub fn parse_reply(content: &str) -> Option<Value> {
    let stripped = strip_think(content);
    let text = stripped.trim();
    let first = text.find('{')?;
    let last = text.rfind('}')?;
    if last < first {
        return None;
    }
    match serde_json::from_str::<Value>(&text[first..=last]).ok()? {
        Value::Object(map) => Some(Value::Object(map)),
        _ => None,
    }
}

/// Remove non-overlapping `<think>.*?</think>` spans, as `re.sub(..., flags=re.S)` does.
fn strip_think(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        let after_open = &rest[start + "<think>".len()..];
        match after_open.find("</think>") {
            Some(end) => {
                out.push_str(&rest[..start]);
                rest = &after_open[end + "</think>".len()..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

fn breaker_allows(state: &AppState) -> bool {
    lock(&state.breaker).allow(Instant::now())
}

fn breaker_record(state: &AppState, ok: bool) {
    lock(&state.breaker).record(ok, Instant::now());
}

/// Positive, finite seconds for a `Duration`, falling back to 20 s for a bad configuration value.
fn llm_timeout(state: &AppState) -> Duration {
    Duration::try_from_secs_f64(state.config.llm_timeout_s).unwrap_or(Duration::from_secs(20))
}

/// One POST and reply parse. `None` on any failure. The caller records the breaker outcome.
async fn request_reply(
    state: &AppState,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Option<Value> {
    let url = format!("{}/chat/completions", state.config.llm_base_url);
    let body = json!({
        "model": state.config.llm_model,
        "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }],
        "max_tokens": max_tokens,
        "response_format": { "type": "json_object" },
        "temperature": 0.7,
        "top_p": 0.8,
        "top_k": 20,
        "min_p": 0.0,
        "presence_penalty": 1.5,
        "repetition_penalty": 1.0,
        "chat_template_kwargs": { "enable_thinking": false, "preserve_thinking": false },
    });
    let mut req = state
        .http
        .post(&url)
        .timeout(llm_timeout(state))
        .json(&body);
    if !state.config.llm_api_key.is_empty() {
        req = req.bearer_auth(&state.config.llm_api_key);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let reply: Value = resp.json().await.ok()?;
    let content = reply
        .get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?;
    let text = match content {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    parse_reply(&text)
}

/// `_chat`: one JSON-mode completion, or `None`. Never raises and never queues.
pub async fn chat(state: &AppState, system: &str, user: &str, max_tokens: u32) -> Option<Value> {
    if state.config.llm_base_url.is_empty() || !breaker_allows(state) {
        return None;
    }
    // Busy means skip, without touching the breaker (Python's locked-semaphore branch).
    let _permit = state.llm_slots.try_acquire().ok()?;
    let parsed = request_reply(state, system, user, max_tokens).await;
    breaker_record(state, parsed.is_some());
    parsed
}

/// Sensor and forecast inputs for one briefing. Each is best effort, as in Python.
async fn gather(state: &AppState, lat: f64, lon: f64) -> (Value, Value, Value, Vec<Value>) {
    let forecast = async {
        forecast_for(state, lat, lon)
            .await
            .unwrap_or_else(|_| json!({}))
    };
    let nowcast = async {
        nowcast_point(state, lat, lon)
            .await
            .map(|r| r.body)
            .unwrap_or_else(|_| json!({}))
    };
    let storms = async {
        let path = state.config.state_dir.join("storms.json");
        tokio::task::spawn_blocking(move || {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .unwrap_or_else(|| json!({}))
        })
        .await
        .unwrap_or_else(|_| json!({}))
    };
    let alerts = async {
        let mut out: Vec<Value> = Vec::new();
        if let Ok(body) = alerts_for(state, lat, lon).await
            && let Some(features) = body.get("features").and_then(Value::as_array)
        {
            {
                for feature in features.iter().filter(|f| f.is_object()) {
                    out.push(
                        feature
                            .get("properties")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    );
                }
            }
        }
        out
    };
    tokio::join!(forecast, nowcast, storms, alerts)
}

/// Removes this request's single-flight entry when the leader finishes, fails or is cancelled.
struct InflightGuard<'a> {
    state: &'a AppState,
    key: String,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        lock(&self.state.inflight).remove(&self.key);
    }
}

/// The briefing body for a leader, or `None` when there is nothing to say.
async fn narrate(state: &AppState, lat: f64, lon: f64, place: &str) -> Option<Value> {
    let (forecast, nowcast, storms, alerts) = gather(state, lat, lon).await;
    truthy(forecast.get("current"))?;
    let storm = incoming_storm(&storms, lat, lon);
    let facts = build_facts(place, &forecast, &nowcast, &alerts, storm, Utc::now());
    let user = dumps_value(&facts, Style::Default, false);
    let reply = chat(state, BRIEFING_SYSTEM, &user, 160).await?;
    truthy(reply.get("body"))?;
    Some(json!({
        "available": true,
        "headline": clean(reply.get("headline"), 80),
        "body": clean(reply.get("body"), 400),
        "generated_at": now_isoformat(),
        "model": state.config.llm_model,
    }))
}

/// `GET /api/briefing?lat=&lon=&place=`.
pub async fn get_briefing(State(state): State<AppState>, uri: Uri) -> Response {
    let raw = uri.query();
    let mut issues = Issues::new();
    let lat = issues.float_in(
        "query",
        "lat",
        query_param(raw, "lat").as_deref(),
        -90.0,
        90.0,
    );
    let lon = issues.float_in(
        "query",
        "lon",
        query_param(raw, "lon").as_deref(),
        -180.0,
        180.0,
    );
    let place_raw = issues.string(
        "query",
        "place",
        query_param(raw, "place").as_deref(),
        Some(""),
        0,
        80,
    );
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon), Some(place_raw)) = (lat, lon, place_raw) else {
        return respond::internal_error();
    };
    if state.config.llm_base_url.is_empty() {
        return unavailable("not_configured");
    }
    if !breaker_allows(&state) {
        return unavailable("llm_unavailable");
    }
    let place = clean_text(&place_raw, 60);
    let key = format!(
        "{},{},{}",
        float_repr(round_digits(lat, 1)),
        float_repr(round_digits(lon, 1)),
        place.to_lowercase()
    );
    if let Some(hit) = state.briefing_cache.get(&key) {
        return ok_json(&hit, 120);
    }

    // One LLM call per area, however many requests ask at once.
    let follower = lock(&state.inflight).get(&key).cloned();
    if let Some(mut rx) = follower {
        let body: Option<Arc<Value>> = match rx.wait_for(|v| v.is_some()).await {
            Ok(v) => (*v).clone(),
            Err(_) => None,
        };
        return match body {
            Some(body) => respond::json(StatusCode::OK, &body, None),
            None => unavailable("llm_unavailable"),
        };
    }

    let (tx, rx) = watch::channel::<Option<Arc<Value>>>(None);
    lock(&state.inflight).insert(key.clone(), rx);
    let _guard = InflightGuard {
        state: &state,
        key: key.clone(),
    };
    let body = narrate(&state, lat, lon, &place).await.map(Arc::new);
    if let Some(body) = &body {
        state.briefing_cache.insert(key.clone(), body.clone());
    }
    tx.send_replace(body.clone());
    drop(tx);
    match body {
        Some(body) => ok_json(&body, 120),
        None => unavailable("llm_unavailable"),
    }
}

/// The `properties` object of the NWS alert named by `id`, or `None` on any failure.
async fn fetch_alert_properties(state: &AppState, id: &str) -> Option<Value> {
    let alert_id = id.rsplit('/').next().unwrap_or(id);
    // D14: the detail base is configurable (NWS_ALERT_DETAIL_BASE). The default is the NWS host.
    let base = state.config.nws_alert_detail_base.trim_end_matches('/');
    let url = format!("{base}/{alert_id}");
    let headers = [
        ("User-Agent", state.config.nws_user_agent.as_str()),
        ("Accept", "application/geo+json"),
    ];
    let fetched = upstream::get(&state.http, &url, &[], &headers, Duration::from_secs(8))
        .await
        .ok()?;
    if !fetched.is_success() {
        return None;
    }
    let body = fetched.json()?;
    let object = body.as_object()?;
    Some(
        object
            .get("properties")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
}

/// `GET /api/alerts/explain?id=`.
pub async fn explain_alert(State(state): State<AppState>, uri: Uri) -> Response {
    let raw = uri.query();
    let mut issues = Issues::new();
    let id = issues.string(
        "query",
        "id",
        query_param(raw, "id").as_deref(),
        None,
        8,
        200,
    );
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let Some(id) = id else {
        return respond::internal_error();
    };
    if state.config.llm_base_url.is_empty() {
        return unavailable("not_configured");
    }
    if !(id.starts_with("urn:oid:") || id.starts_with("https://api.weather.gov/alerts/")) {
        return unavailable("bad_id");
    }
    let key = format!("alert:{id}");
    if let Some(hit) = state.briefing_cache.get(&key) {
        return ok_json(&hit, 600);
    }
    let Some(props) = fetch_alert_properties(&state, &id).await else {
        return unavailable("alert_unavailable");
    };
    let text = json!({
        "event": props.get("event").cloned().unwrap_or(Value::Null),
        "area": clean(props.get("areaDesc"), 300),
        "headline": props.get("headline").cloned().unwrap_or(Value::Null),
        "description": clean(props.get("description"), 1500),
        "instruction": clean(props.get("instruction"), 600),
        "ends": truthy(props.get("ends")).or_else(|| props.get("expires")).cloned().unwrap_or(Value::Null),
    });
    let user = dumps_value(&text, Style::Default, false);
    let reply = chat(&state, EXPLAIN_SYSTEM, &user, 140).await;
    let Some(reply) = reply.filter(|r| truthy(r.get("what")).is_some()) else {
        return unavailable("llm_unavailable");
    };
    let body = Arc::new(json!({
        "available": true,
        "what": clean(reply.get("what"), 300),
        "do": clean(reply.get("do"), 300),
        "model": state.config.llm_model,
    }));
    state.briefing_cache.insert(key, body.clone());
    ok_json(&body, 600)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn clean_collapses_and_truncates_by_characters() {
        assert_eq!(clean_text("  a \n b  ", 10), "a b");
        assert_eq!(clean_text("abcdef", 3), "abc");
        assert_eq!(clean(Some(&json!(0)), 10), "");
    }

    #[test]
    fn wmo_labels_and_unknown() {
        assert_eq!(wmo_label(Some(&json!(61))), "light rain");
        assert_eq!(wmo_label(Some(&json!(61.0))), "light rain");
        assert_eq!(wmo_label(Some(&json!(61.5))), "unknown");
        assert_eq!(wmo_label(None), "unknown");
    }

    #[test]
    fn nowcast_summary_dry_and_wet() {
        let dry = vec![json!({"timestamp": "2026-10-13T12:00:00+00:00", "dbz": 3})];
        assert_eq!(summarize_nowcast(&dry, 0), "dry for the next hour");
        let wet = vec![
            json!({"timestamp": "2026-10-13T12:00:00+00:00", "dbz": 5}),
            json!({"timestamp": "2026-10-13T12:05:00+00:00", "dbz": 20}),
            json!({"timestamp": "2026-10-13T12:10:00+00:00", "dbz": 38}),
            json!({"timestamp": "2026-10-13T12:15:00+00:00", "dbz": 22}),
            json!({"timestamp": "2026-10-13T12:20:00+00:00", "dbz": 4}),
        ];
        assert_eq!(
            summarize_nowcast(&wet, 0),
            "rain from about 12:05 PM, peaking near 12:10 PM at 38 dBZ, ending around 12:20 PM"
        );
    }

    #[test]
    fn storm_inside_radius_is_found_with_earliest_minute() {
        let storms = json!({"features": [{
            "geometry": {"type": "Point", "coordinates": [-85.0, 43.0]},
            "properties": {
                "track_history": [[0, -85.1, 42.9], [0, -85.05, 42.95], [0, -85.0, 43.0]],
                "tracking_confidence": 0.9,
                "peak_dbz": 57.4,
                "tracking_vector": {"east_kmh": 0, "north_kmh": -40, "speed_kmh": 40}
            }
        }]});
        // Place about 20 km south of the cell. The cell moves south at 40 km/h, so it is within 12 km
        // from minute 15 onward, and minute 10 is still 13 km away.
        let hit = incoming_storm(&storms, 42.82, -85.0).expect("hit");
        assert_eq!(hit["peak_dbz"], json!(57));
        assert_eq!(hit["speed_mph"], json!(25));
        assert!(hit["minutes"].as_i64().expect("minutes") > 0);
    }

    #[test]
    fn untracked_cell_only_counts_now() {
        let storms = json!({"features": [{
            "geometry": {"type": "Point", "coordinates": [-85.0, 43.0]},
            "properties": {"track_history": [], "tracking_confidence": 0.0,
                           "tracking_vector": {"north_kmh": -40}}
        }]});
        // About 22 km south of the cell: a tracked cell would reach it, an untracked one does not move.
        assert!(incoming_storm(&storms, 42.8, -85.0).is_none());
        // At the cell itself the current-position check (minute 0) always counts.
        assert!(incoming_storm(&storms, 43.0, -85.0).is_some());
    }

    #[test]
    fn reply_parser_strips_think_and_requires_object() {
        let text = "<think>plan</think> ```json {\"headline\": \"Hi\", \"body\": \"x\"} ```";
        assert_eq!(parse_reply(text).expect("object")["headline"], json!("Hi"));
        assert!(parse_reply("[1,2]").is_none());
        assert!(parse_reply("no braces").is_none());
    }

    #[test]
    fn facts_have_python_key_order_and_sky_labels() {
        let now = Utc
            .with_ymd_and_hms(2026, 10, 13, 12, 0, 0)
            .single()
            .expect("time");
        let forecast = json!({
            "utc_offset_seconds": -14400,
            "current": {"temperature_2m": 61.4, "weather_code": 61, "wind_speed_10m": 8.2},
            "hourly": {"time": ["2026-10-13T08:00", "2026-10-13T09:00", "2026-10-13T10:00", "2026-10-13T11:00"],
                       "temperature_2m": [50, 52, 55, 58], "weather_code": [0, 1, 61, 3],
                       "precipitation": [0.0, 0.01, 0.02, 0.0]},
            "daily": {"temperature_2m_max": [66.2], "temperature_2m_min": [52.0], "precipitation_sum": [0.3]}
        });
        let facts = build_facts("", &forecast, &json!({}), &[], None, now);
        let keys: Vec<&str> = facts
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec![
                "place",
                "local_time",
                "now",
                "next_hour_radar",
                "incoming_storm",
                "next_12_hours",
                "today",
                "active_alerts"
            ]
        );
        assert_eq!(facts["place"], json!("this spot"));
        assert_eq!(facts["now"]["sky"], json!("light rain"));
        assert_eq!(facts["now"]["temp_f"], json!(61));
        assert_eq!(facts["next_hour_radar"], json!("radar nowcast unavailable"));
        // 2026-10-13 is a Tuesday; 12:00 UTC at UTC-4 is 8:00 AM.
        assert_eq!(facts["local_time"], json!("Tuesday 8:00 AM"));
    }
}
