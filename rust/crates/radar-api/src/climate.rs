//! `GET /api/climate/white-christmas`: the chance of a white Christmas at NCEI normals stations.
//!
//! The data is `assets/white-christmas.json`, built once by `rust/scripts/build-white-christmas.py`
//! from the NCEI 1991-2020 daily station normals (`DLY-SNWD-PCTALL-GE001WI` on Dec 25: percent of
//! years with at least 1 inch of snow on the ground). It never changes, so it is embedded.
//!
//! - No query: a GeoJSON `FeatureCollection` of every station, for the map dots.
//! - `?lat=&lon=`: the nearest station and its distance in miles. No interpolation.

use std::sync::LazyLock;

use axum::body::Bytes;
use axum::http::{StatusCode, Uri};
use axum::response::Response;
use serde_json::{Value, json};

use crate::params::{Issues, query_param};
use crate::respond;

const ASSET: &str = include_str!("../assets/white-christmas.json");
const EARTH_RADIUS_MI: f64 = 3958.8;
/// The data is static; a day of client caching is plenty.
const MAX_AGE_S: u64 = 86_400;

#[derive(Debug, Clone, PartialEq)]
struct Station {
    id: String,
    name: String,
    lat: f64,
    lon: f64,
    pct: f64,
    years: u64,
}

struct Data {
    source: String,
    stations: Vec<Station>,
}

static DATA: LazyLock<Data> = LazyLock::new(|| {
    let doc: Value = serde_json::from_str(ASSET).expect("embedded white-christmas JSON is valid");
    let stations = doc["stations"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    Some(Station {
                        id: r.get(0)?.as_str()?.to_owned(),
                        name: r.get(1)?.as_str()?.to_owned(),
                        lat: r.get(2)?.as_f64()?,
                        lon: r.get(3)?.as_f64()?,
                        pct: r.get(4)?.as_f64()?,
                        years: r.get(5)?.as_u64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Data {
        source: doc["source"].as_str().unwrap_or_default().to_owned(),
        stations,
    }
});

static ALL_BODY: LazyLock<Bytes> = LazyLock::new(|| {
    let features: Vec<Value> = DATA
        .stations
        .iter()
        .map(|s| {
            json!({
                "type": "Feature",
                "geometry": { "type": "Point", "coordinates": [s.lon, s.lat] },
                "properties": { "id": s.id, "name": s.name, "pct": s.pct, "years": s.years },
            })
        })
        .collect();
    respond::encode(&json!({
        "type": "FeatureCollection",
        "source": DATA.source,
        "features": features,
    }))
});

/// Great-circle distance in miles.
fn distance_mi(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_MI * a.sqrt().min(1.0).asin()
}

fn nearest(lat: f64, lon: f64) -> Option<(&'static Station, f64)> {
    DATA.stations
        .iter()
        .map(|s| (s, distance_mi(lat, lon, s.lat, s.lon)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// `GET /api/climate/white-christmas[?lat=&lon=]`.
pub async fn get_white_christmas(uri: Uri) -> Response {
    let raw = uri.query();
    let (lat_raw, lon_raw) = (query_param(raw, "lat"), query_param(raw, "lon"));
    if lat_raw.is_none() && lon_raw.is_none() {
        return respond::json_bytes(
            StatusCode::OK,
            ALL_BODY.clone(),
            Some(&respond::public_max_age(MAX_AGE_S)),
        );
    }
    let mut issues = Issues::new();
    let lat = issues.float_in("query", "lat", lat_raw.as_deref(), -90.0, 90.0);
    let lon = issues.float_in("query", "lon", lon_raw.as_deref(), -180.0, 180.0);
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return respond::internal_error();
    };
    let station = nearest(lat, lon).map(|(s, d)| {
        json!({
            "id": s.id,
            "name": s.name,
            "lat": s.lat,
            "lon": s.lon,
            "pct": s.pct,
            "years": s.years,
            "distance_mi": (d * 10.0).round() / 10.0,
        })
    });
    respond::json(
        StatusCode::OK,
        &json!({ "source": DATA.source, "station": station }),
        Some(&respond::public_max_age(MAX_AGE_S)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(id: &str) -> &'static Station {
        DATA.stations
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("station {id} is in the asset"))
    }

    #[test]
    fn asset_has_every_station_and_percent_values() {
        assert!(DATA.stations.len() > 4500, "{}", DATA.stations.len());
        assert!(DATA.source.contains("1991-2020"));
        assert!(DATA.stations.iter().all(|s| (0.0..=100.0).contains(&s.pct)));
    }

    /// The NCEI docs say "tenths of a percent", but the values are whole percent. These three
    /// match NOAA's published 1991-2020 white Christmas odds (NWS Detroit's Christmas page links
    /// climate.gov: 34.7%; Duluth 92.2% and Minneapolis 73.6% from the same NOAA normals).
    #[test]
    fn percent_scale_matches_three_known_cities() {
        assert_eq!(station("USW00094847").pct, 34.7, "Detroit Metro");
        assert_eq!(station("USW00014913").pct, 92.2, "Duluth");
        assert_eq!(station("USW00014922").pct, 73.6, "Minneapolis/St Paul");
    }

    #[test]
    fn nearest_station_and_distance() {
        // Downtown Grand Rapids: the airport station is about 6 miles south-east.
        let (s, d) = nearest(42.9634, -85.6681).unwrap();
        assert!(s.name.contains("GRAND RAPIDS"), "{}", s.name);
        assert!(d < 10.0, "{d}");
        let dtw = station("USW00094847");
        assert!(distance_mi(dtw.lat, dtw.lon, dtw.lat, dtw.lon).abs() < 1e-9);
        // Detroit to Grand Rapids is about 130 miles.
        let grr = station("USW00094860");
        let d = distance_mi(dtw.lat, dtw.lon, grr.lat, grr.lon);
        assert!((120.0..140.0).contains(&d), "{d}");
    }

    #[test]
    fn all_body_is_geojson_points() {
        let v: Value = serde_json::from_slice(&ALL_BODY).unwrap();
        assert_eq!(v["type"], "FeatureCollection");
        let f = &v["features"][0];
        assert_eq!(f["geometry"]["type"], "Point");
        assert!(f["properties"]["pct"].is_number());
        assert_eq!(v["features"].as_array().unwrap().len(), DATA.stations.len());
    }
}
