//! `GET /api/forecast/{lat}/{lon}`: the Open-Meteo passthrough with a rounded-cell cache (spec 5.2).
//!
//! The point is rounded to one decimal (about 11 km). The upstream request and the cache key both
//! use the rounded values. Concurrent misses for one cell share one upstream request. The 200 body
//! is the Open-Meteo JSON, unchanged, cached encoded. Failures are
//! `502 {"error":"upstream_unavailable"}` with **no** `Cache-Control`. A 200 whose body is not JSON
//! is also a 502 (defect D5 fix; Python returned 500). A body whose current temperature is null (no model
//! sync yet on a fresh stack) is served `no-store` and not cached, so data shows up once it lands.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use radar_core::pycompat::float_repr;
use radar_core::time::round_digits;
use serde_json::Value;
use std::sync::atomic::Ordering;

use crate::params::Issues;
use crate::respond;
use crate::state::AppState;
use crate::upstream;

/// Timeout for one Open-Meteo request (client default 15 s).
const TIMEOUT: Duration = Duration::from_secs(15);

const CURRENT: &str = "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m,dew_point_2m,surface_pressure,precipitation";
const HOURLY: &str = "temperature_2m,precipitation,precipitation_probability,weather_code,wind_speed_10m,relative_humidity_2m,apparent_temperature";
const MINUTELY_15: &str = "temperature_2m,relative_humidity_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m";
const DAILY: &str = "temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,precipitation_probability_max,uv_index_max,sunrise,sunset";

/// Why a forecast could not be produced.
#[derive(Debug)]
pub enum ForecastError {
    /// `lat`/`lon` outside the valid ranges (HTTP 422 `lat/lon out of range`).
    OutOfRange,
    /// Upstream failed or returned a non-JSON body (HTTP 502 `upstream_unavailable`).
    Upstream,
    /// Upstream answered, but without data yet (HTTP 200, `no-store`, not cached).
    NoData(Bytes),
}

/// The rounded cell for a point, as Python computes it: `round(x, 1)`.
#[must_use]
pub fn cell_key(lat: f64, lon: f64) -> (f64, f64, String) {
    let glat = round_digits(lat, 1);
    let glon = round_digits(lon, 1);
    let key = format!("{},{}", float_repr(glat), float_repr(glon));
    (glat, glon, key)
}

/// Forecast body for a point, parsed, counted as one `forecast_requests_total` request (the
/// briefing path).
///
/// # Errors
/// [`ForecastError::OutOfRange`] for a bad point, [`ForecastError::Upstream`] for any upstream failure.
pub async fn forecast_for(state: &AppState, lat: f64, lon: f64) -> Result<Value, ForecastError> {
    state
        .metrics
        .counters
        .forecast_requests_total
        .fetch_add(1, Ordering::Relaxed);
    let body = match forecast_lookup(state, lat, lon).await {
        Ok(body) | Err(ForecastError::NoData(body)) => body,
        Err(e) => return Err(e),
    };
    serde_json::from_slice(&body).map_err(|_| ForecastError::Upstream)
}

/// Encoded forecast body for a point, from the cache or Open-Meteo. Does not count the request:
/// the caller does, exactly once, so `GET /api/forecast` is counted once (before validation, as in
/// Python). A request that waited on another request's upstream call counts as a cache hit.
///
/// # Errors
/// [`ForecastError::OutOfRange`] for a bad point, [`ForecastError::Upstream`] for any upstream failure,
/// [`ForecastError::NoData`] (with the body) when Open-Meteo has no data for the point yet.
pub async fn forecast_lookup(state: &AppState, lat: f64, lon: f64) -> Result<Bytes, ForecastError> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err(ForecastError::OutOfRange);
    }
    let (glat, glon, key) = cell_key(lat, lon);
    let entry = state
        .forecast_cache
        .entry(key)
        .or_try_insert_with(fetch(state, glat, glon))
        .await
        .map_err(|e| match &*e {
            ForecastError::NoData(body) => ForecastError::NoData(body.clone()),
            _ => ForecastError::Upstream,
        })?;
    if !entry.is_fresh() {
        state
            .metrics
            .counters
            .forecast_cache_hits_total
            .fetch_add(1, Ordering::Relaxed);
    }
    Ok(entry.into_value())
}

/// One Open-Meteo request for a rounded cell, encoded.
async fn fetch(state: &AppState, glat: f64, glon: f64) -> Result<Bytes, ForecastError> {
    let query: Vec<(&str, String)> = vec![
        ("latitude", float_repr(glat)),
        ("longitude", float_repr(glon)),
        ("current", CURRENT.into()),
        ("hourly", HOURLY.into()),
        ("minutely_15", MINUTELY_15.into()),
        ("daily", DAILY.into()),
        ("temperature_unit", "fahrenheit".into()),
        ("wind_speed_unit", "mph".into()),
        ("precipitation_unit", "inch".into()),
        ("timezone", "auto".into()),
        ("forecast_days", "7".into()),
    ];
    let fetched = upstream::get(
        &state.http,
        &state.config.open_meteo_base,
        &query,
        &[],
        TIMEOUT,
    )
    .await;
    let body = match fetched {
        Ok(f) if f.is_success() => f.json(),
        _ => None,
    };
    let Some(data) = body else {
        state
            .metrics
            .counters
            .forecast_upstream_errors_total
            .fetch_add(1, Ordering::Relaxed);
        return Err(ForecastError::Upstream);
    };
    let encoded = respond::encode(&data);
    if data
        .pointer("/current/temperature_2m")
        .is_some_and(Value::is_null)
    {
        return Err(ForecastError::NoData(encoded));
    }
    Ok(encoded)
}

/// `GET /api/forecast/{lat}/{lon}`.
pub async fn get_forecast(
    State(state): State<AppState>,
    Path((lat_raw, lon_raw)): Path<(String, String)>,
) -> Response {
    state
        .metrics
        .counters
        .forecast_requests_total
        .fetch_add(1, Ordering::Relaxed);
    let mut issues = Issues::new();
    let lat = issues.float("path", "lat", Some(&lat_raw));
    let lon = issues.float("path", "lon", Some(&lon_raw));
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return respond::internal_error();
    };
    let ttl = state.config.forecast_ttl_s;
    let resp = match forecast_lookup(&state, lat, lon).await {
        Ok(data) => respond::json_bytes(StatusCode::OK, data, Some(&respond::public_max_age(ttl))),
        Err(ForecastError::OutOfRange) => {
            respond::detail(StatusCode::UNPROCESSABLE_ENTITY, "lat/lon out of range")
        }
        Err(ForecastError::Upstream) => respond::upstream_unavailable(),
        Err(ForecastError::NoData(data)) => {
            respond::json_bytes(StatusCode::OK, data, Some("no-store"))
        }
    };
    resp.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_key_rounds_to_one_decimal_like_python() {
        // Spec 5.2 [V]: 42.96 -> 43.0, -85.67 -> -85.7.
        let (glat, glon, key) = cell_key(42.96, -85.67);
        assert_eq!(glat, 43.0);
        assert_eq!(glon, -85.7);
        assert_eq!(key, "43.0,-85.7");
    }
}
