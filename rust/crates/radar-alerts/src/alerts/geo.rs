//! Point-in-polygon for NWS alert geometry (spec-ingest-aux section 10.5, Python `_point_in_geojson`).
//!
//! Ray casting over GeoJSON `[lng, lat]` positions. Holes subtract, and a malformed hole counts as "not in hole".
//! A malformed outer ring, or any malformed vertex in the outer ring, makes the point outside. Coordinates may be
//! JSON numbers, booleans or numeric strings, which is what Python's `float()` accepts.

use radar_storms::numeric::py_float;
use serde_json::{Map, Value};

/// True when `(lng, lat)` is inside `geometry`. Supports `Polygon` and `MultiPolygon`. Anything else, including an
/// empty object, is outside.
#[must_use]
pub fn point_in_geojson(lng: f64, lat: f64, geometry: &Map<String, Value>) -> bool {
    if geometry.is_empty() {
        return false;
    }
    let coordinates = geometry.get("coordinates");
    match geometry.get("type").and_then(Value::as_str) {
        Some("Polygon") => coordinates.is_some_and(|rings| point_in_polygon(lng, lat, rings)),
        Some("MultiPolygon") => match coordinates.and_then(Value::as_array) {
            Some(polygons) => polygons
                .iter()
                .any(|polygon| point_in_polygon(lng, lat, polygon)),
            None => false,
        },
        _ => false,
    }
}

/// `rings[0]` is the outer ring. The point must be inside it and outside every hole.
fn point_in_polygon(lng: f64, lat: f64, polygon: &Value) -> bool {
    let Some(rings) = polygon.as_array().filter(|r| !r.is_empty()) else {
        return false;
    };
    if !point_in_ring(lng, lat, &rings[0]) {
        return false;
    }
    !rings[1..].iter().any(|hole| point_in_ring(lng, lat, hole))
}

/// Even-odd ray cast along the longitude axis. Rings with fewer than four positions are not rings.
fn point_in_ring(lng: f64, lat: f64, ring: &Value) -> bool {
    let Some(points) = ring.as_array().filter(|r| r.len() >= 4) else {
        return false;
    };
    let Some(mut prev) = coordinate(&points[points.len() - 1]) else {
        return false;
    };
    let mut inside = false;
    for point in points {
        let Some(curr) = coordinate(point) else {
            return false;
        };
        let (curr_lng, curr_lat) = curr;
        let (prev_lng, prev_lat) = prev;
        let crosses = (curr_lat > lat) != (prev_lat > lat);
        if crosses {
            let intersect_lng =
                (prev_lng - curr_lng) * (lat - curr_lat) / (prev_lat - curr_lat) + curr_lng;
            if lng < intersect_lng {
                inside = !inside;
            }
        }
        prev = curr;
    }
    inside
}

/// `(lng, lat)` from a position: an array of at least two values, each convertible to a float.
fn coordinate(point: &Value) -> Option<(f64, f64)> {
    let values = point.as_array()?;
    if values.len() < 2 {
        return None;
    }
    Some((py_float(&values[0])?, py_float(&values[1])?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn polygon(coords: Value) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("type".into(), json!("Polygon"));
        m.insert("coordinates".into(), coords);
        m
    }

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
        json!([[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]])
    }

    #[test]
    fn point_with_hole_is_out_of_polygon_and_inside_outer_is_in() {
        let g = polygon(json!([
            square(-98.0, 35.0, -96.0, 37.0),
            square(-97.5, 35.5, -96.5, 36.5)
        ]));
        assert!(
            point_in_geojson(-97.9, 35.9, &g),
            "inside outer, outside hole"
        );
        assert!(!point_in_geojson(-97.0, 36.0, &g), "inside the hole");
        assert!(!point_in_geojson(-95.0, 36.0, &g), "outside the outer ring");
    }

    #[test]
    fn multipolygon_is_in_when_inside_any_part() {
        let g: Map<String, Value> = serde_json::from_value(json!({
            "type": "MultiPolygon",
            "coordinates": [
                [square(-98.0, 35.0, -96.0, 37.0)],
                [square(-80.0, 30.0, -79.0, 31.0)]
            ]
        }))
        .unwrap();
        assert!(point_in_geojson(-79.5, 30.5, &g));
        assert!(point_in_geojson(-97.0, 36.0, &g));
        assert!(!point_in_geojson(-90.0, 33.0, &g), "outside all parts");
    }

    #[test]
    fn empty_unknown_and_malformed_geometries_are_outside() {
        assert!(!point_in_geojson(0.0, 0.0, &Map::new()));
        let point: Map<String, Value> =
            serde_json::from_value(json!({"type": "Point", "coordinates": [0, 0]})).unwrap();
        assert!(!point_in_geojson(0.0, 0.0, &point));
        assert!(!point_in_geojson(0.0, 0.0, &polygon(json!([]))));
        assert!(
            !point_in_geojson(0.0, 0.0, &polygon(json!([[[0, 0], [1, 0], [1, 1]]]))),
            "short ring"
        );
        assert!(
            !point_in_geojson(
                0.5,
                0.5,
                &polygon(json!([[[0, 0], [1, 0], null, [1, 1], [0, 0]]]))
            ),
            "null vertex"
        );
    }

    #[test]
    fn numeric_strings_are_accepted_like_python_float() {
        let g = polygon(json!([[
            ["0", "0"],
            ["2", "0"],
            ["2", "2"],
            ["0", "2"],
            ["0", "0"]
        ]]));
        assert!(point_in_geojson(1.0, 1.0, &g));
    }

    #[test]
    fn malformed_hole_counts_as_not_in_hole() {
        let g = polygon(json!([square(0.0, 0.0, 4.0, 4.0), [[1, 1], [2, 1]]]));
        assert!(point_in_geojson(1.5, 1.5, &g));
    }

    #[test]
    fn ray_cast_matches_hand_checked_points_on_a_triangle() {
        let g = polygon(json!([[[0, 0], [10, 0], [0, 10], [0, 0]]]));
        assert!(point_in_geojson(2.0, 2.0, &g));
        assert!(!point_in_geojson(8.0, 8.0, &g));
        assert!(!point_in_geojson(-1.0, 2.0, &g));
    }
}
