//! `GET /api/palettes`: the tile renderer's palette tables, so client legends use the same colors.
//!
//! Body: `{"<palette>": {"<color key>": {"unit", "ranges": [{"min", "max", "rgba"}], ...}}}`. The files
//! are the ones `radar_tiler::palettes` embeds; `_comment` keys are dropped.

use std::sync::LazyLock;

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Map, Value};

use crate::respond;

const SOURCES: [(&str, &str); 3] = [
    (
        "classic",
        include_str!("../../radar-tiler/assets/classic.json"),
    ),
    ("muted", include_str!("../../radar-tiler/assets/muted.json")),
    ("vivid", include_str!("../../radar-tiler/assets/vivid.json")),
];

static BODY: LazyLock<Bytes> = LazyLock::new(|| {
    let mut out = Map::new();
    for (name, text) in SOURCES {
        let mut table: Map<String, Value> =
            serde_json::from_str(text).expect("embedded palette JSON is valid");
        table.retain(|key, _| !key.starts_with('_'));
        out.insert(name.to_owned(), Value::Object(table));
    }
    respond::encode(&Value::Object(out))
});

/// `GET /api/palettes`.
pub async fn get_palettes() -> Response {
    respond::json_bytes(
        StatusCode::OK,
        BODY.clone(),
        Some(&respond::public_max_age(3600)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_has_the_layer_color_keys_and_no_comments() {
        let v: Value = serde_json::from_slice(&BODY).unwrap();
        for (name, _) in SOURCES {
            let table = v[name].as_object().unwrap();
            assert!(table.keys().all(|k| !k.starts_with('_')));
            for key in [
                "reflectivity",
                "temperature",
                "precip_accum",
                "cloud_cover",
                "wind_speed",
            ] {
                let ranges = table[key]["ranges"].as_array().unwrap();
                assert!(!ranges.is_empty(), "{name}.{key}");
                assert_eq!(ranges[0]["rgba"].as_array().unwrap().len(), 4);
            }
        }
    }
}
