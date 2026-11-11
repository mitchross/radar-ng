//! Read-only famous-storm replay routes (design famous-storm-replay section 4).
//!
//! - `GET /api/replays`: the catalogue of ready cases, one newest ready revision each.
//! - `GET /api/replays/{id}/{revision}/manifest.json` and `/warnings.geojson`: the stored bytes.
//! - `GET /replays/{id}/{revision}/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png`: immutable PNGs.
//!
//! A revision is served only when `revision.json`, the manifest and the warnings all pass
//! [`load`]. Anything else is a 404: never a live fallback, never a partial or pilot import.
//! Case ids come from the importer's fixed case list; revisions and path parts are validated
//! before any file is touched, and every file read is size-capped and must be a regular file.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use radar_replay::case::{self, CASE_PALETTE, CASE_ZOOMS, CaseDefinition};
use serde_json::{Value, json};

use crate::manifest::{etag_of, if_none_match_matches};
use crate::respond;
use crate::state::AppState;

const CATALOGUE_SCHEMA_VERSION: u64 = 1;
const MAX_REVISION_BYTES: u64 = 256 * 1024;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_WARNINGS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TILE_BYTES: u64 = 1024 * 1024;
const MAX_REVISIONS_PER_CASE: usize = 64;
const CATALOGUE_CC: &str = "public, max-age=60";
const REVISION_CC: &str = "public, max-age=3600";
const TILE_CC: &str = "public, max-age=31536000, immutable";

/// The validated bytes of one ready revision.
struct Ready {
    case: &'static CaseDefinition,
    revision: String,
    metadata: Value,
    manifest: Vec<u8>,
    warnings: Vec<u8>,
    layers: Vec<String>,
    stamp: SystemTime,
}

/// True for a safe revision name (the importer's own rule).
fn valid_revision(revision: &str) -> bool {
    revision.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && revision.len() <= 64
        && revision
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// True when a `/tiles/...` request path (relative to the tile root) reaches the replay root.
pub(crate) fn is_private_tile_path(rel: &str) -> bool {
    let decoded = percent_encoding::percent_decode_str(rel).decode_utf8_lossy();
    decoded
        .split('/')
        .find(|s| !s.is_empty() && *s != ".")
        .is_some_and(|s| s.eq_ignore_ascii_case(case_root_name()))
}

fn case_root_name() -> &'static str {
    radar_replay::quota::REPLAYS_DIR
}

fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > cap {
        return None;
    }
    fs::read(path).ok()
}

fn parse(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice(bytes).ok()
}

fn bounds_match(value: &Value, expected: &[f64; 4]) -> bool {
    value.as_array().is_some_and(|a| {
        a.len() == 4
            && a.iter()
                .zip(expected)
                .all(|(v, e)| v.as_f64().is_some_and(|v| (v - e).abs() < 1e-9))
    })
}

fn revision_ok(meta: &Value, case: &CaseDefinition, revision: &str) -> bool {
    let text = |key: &str| meta.get(key).and_then(Value::as_str);
    let sources_ok = meta
        .get("sources")
        .and_then(Value::as_array)
        .is_some_and(|s| {
            !s.is_empty()
                && s.iter().all(|s| {
                    s.get("name").and_then(Value::as_str).is_some()
                        && s.get("base_url").and_then(Value::as_str).is_some()
                })
        });
    let warnings = meta.get("warnings");
    meta.get("schema_version").and_then(Value::as_u64) == Some(1)
        && text("case") == Some(case.id)
        && text("revision") == Some(revision)
        && text("status") == Some("tiles_complete")
        && text("scope") == Some("case")
        && text("manifest") == Some("manifest.json")
        && text("start") == Some(case.start)
        && text("end") == Some(case.end)
        && text("timezone") == Some(case.timezone)
        && meta.get("public_ready") == Some(&json!(true))
        && meta.get("coverage_reviewed") == Some(&json!(true))
        && meta
            .get("bounds")
            .is_some_and(|b| bounds_match(b, &case.bounds))
        && sources_ok
        && warnings
            .and_then(|w| w.get("status"))
            .and_then(Value::as_str)
            == Some("complete")
        && warnings.and_then(|w| w.get("file")).and_then(Value::as_str) == Some("warnings.geojson")
}

fn manifest_layers(manifest: &Value, case: &CaseDefinition, revision: &str) -> Option<Vec<String>> {
    let template = format!(
        "/replays/{}/{revision}/tiles/{{layer}}/{{palette}}/{{path}}/{{z}}/{{x}}/{{y}}.png",
        case.id
    );
    let same = |v: &Value, key: &str, want: &str| v.get(key).and_then(Value::as_str) == Some(want);
    if manifest.get("schema_version").and_then(Value::as_u64) != Some(2)
        || !same(manifest, "tile_url_template", &template)
        || !same(manifest, "revision", revision)
        || !manifest.get("case").is_some_and(|c| same(c, "id", case.id))
    {
        return None;
    }
    let layers = manifest.get("layers")?.as_object()?;
    let known = case.layers();
    if layers.is_empty() || layers.keys().any(|k| !known.iter().any(|l| l.layer == k)) {
        return None;
    }
    let mut names: Vec<String> = layers.keys().cloned().collect();
    names.sort();
    Some(names)
}

fn warnings_ok(warnings: &Value, case: &CaseDefinition) -> bool {
    warnings.get("type").and_then(Value::as_str) == Some("FeatureCollection")
        && warnings.get("schema_version").and_then(Value::as_u64) == Some(1)
        && warnings.get("case_id").and_then(Value::as_str) == Some(case.id)
        && warnings.get("start").and_then(Value::as_str) == Some(case.start)
        && warnings.get("end").and_then(Value::as_str) == Some(case.end)
        && warnings.get("features").is_some_and(Value::is_array)
}

/// Validate and read one revision under `tile_root`; `None` for anything not publicly ready.
fn load(tile_root: &Path, id: &str, revision: &str) -> Option<Ready> {
    let case = case::find(id).ok()?;
    if !valid_revision(revision) {
        return None;
    }
    let dir: PathBuf = radar_replay::quota::replay_root(tile_root)
        .join(case.id)
        .join(revision);
    if !fs::symlink_metadata(&dir).ok()?.is_dir() {
        return None;
    }
    let meta_path = dir.join("revision.json");
    let stamp = fs::metadata(&meta_path).ok()?.modified().ok()?;
    let metadata = parse(&read_capped(&meta_path, MAX_REVISION_BYTES)?)?;
    if !revision_ok(&metadata, case, revision) {
        return None;
    }
    let manifest = read_capped(&dir.join("manifest.json"), MAX_MANIFEST_BYTES)?;
    let layers = manifest_layers(&parse(&manifest)?, case, revision)?;
    let warnings = read_capped(&dir.join("warnings.geojson"), MAX_WARNINGS_BYTES)?;
    if !warnings_ok(&parse(&warnings)?, case) {
        return None;
    }
    Some(Ready {
        case,
        revision: revision.to_string(),
        metadata,
        manifest,
        warnings,
        layers,
        stamp,
    })
}

fn catalogue_entry(ready: &Ready) -> Value {
    let case = ready.case;
    let base = format!("/api/replays/{}/{}", case.id, ready.revision);
    json!({
        "id": case.id,
        "title": case.title,
        "summary": case.summary,
        "start": case.start,
        "end": case.end,
        "timezone": case.timezone,
        "bounds": case.bounds,
        "revision": ready.revision,
        "layers": ready.layers,
        "manifest_url": format!("{base}/manifest.json"),
        "warnings_url": format!("{base}/warnings.geojson"),
        "sources": ready.metadata["sources"],
    })
}

fn newest_ready(tile_root: &Path, case: &CaseDefinition) -> Option<Ready> {
    let dir = radar_replay::quota::replay_root(tile_root).join(case.id);
    let mut names: Vec<String> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| valid_revision(n))
        .collect();
    names.sort();
    names.truncate(MAX_REVISIONS_PER_CASE);
    names
        .iter()
        .filter_map(|n| load(tile_root, case.id, n))
        .max_by(|a, b| (a.stamp, &a.revision).cmp(&(b.stamp, &b.revision)))
}

fn build_catalogue(tile_root: &Path) -> Value {
    let mut entries: Vec<Value> = case::CASES
        .iter()
        .filter_map(|c| newest_ready(tile_root, c))
        .map(|r| catalogue_entry(&r))
        .collect();
    entries.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    json!({ "schema_version": CATALOGUE_SCHEMA_VERSION, "cases": entries })
}

fn not_found() -> Response {
    respond::detail(StatusCode::NOT_FOUND, "Not Found")
}

fn unavailable() -> Response {
    respond::detail(StatusCode::SERVICE_UNAVAILABLE, "replay unavailable")
}

/// A 200 with `ETag`, or a 304 when `If-None-Match` matches (same rule as the live manifest).
fn conditional(headers: &HeaderMap, body: Bytes, content_type: &'static str, cc: &str) -> Response {
    let etag = etag_of(&body);
    let mut out = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        out.insert(header::ETAG, v);
    }
    respond::insert_cc(&mut out, cc);
    let inm = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    if if_none_match_matches(inm, &etag) {
        return (StatusCode::NOT_MODIFIED, out).into_response();
    }
    out.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    (StatusCode::OK, out, body).into_response()
}

/// `GET /api/replays`.
pub async fn get_catalogue(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let root = state.config.tile_dir.clone();
    match tokio::task::spawn_blocking(move || build_catalogue(&root)).await {
        Ok(value) => conditional(
            &headers,
            respond::encode(&value),
            "application/json",
            CATALOGUE_CC,
        ),
        Err(_) => unavailable(),
    }
}

async fn ready(state: &AppState, id: String, revision: String) -> Result<Option<Ready>, Response> {
    let root = state.config.tile_dir.clone();
    tokio::task::spawn_blocking(move || load(&root, &id, &revision))
        .await
        .map_err(|_| unavailable())
}

/// `GET /api/replays/{id}/{revision}/manifest.json`.
pub async fn get_manifest(
    State(state): State<AppState>,
    UrlPath((id, revision)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    match ready(&state, id, revision).await {
        Ok(Some(r)) => conditional(
            &headers,
            Bytes::from(r.manifest),
            "application/json",
            REVISION_CC,
        ),
        Ok(None) => not_found(),
        Err(resp) => resp,
    }
}

/// `GET /api/replays/{id}/{revision}/warnings.geojson`.
pub async fn get_warnings(
    State(state): State<AppState>,
    UrlPath((id, revision)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    match ready(&state, id, revision).await {
        Ok(Some(r)) => conditional(
            &headers,
            Bytes::from(r.warnings),
            "application/geo+json",
            REVISION_CC,
        ),
        Ok(None) => not_found(),
        Err(resp) => resp,
    }
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.len() <= 8 && s.bytes().all(|b| b.is_ascii_digit())
}

fn frame_path_ok(s: &str) -> bool {
    s.len() <= 40
        && s.starts_with(|c: char| c.is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'-' | b':' | b'T' | b'Z' | b'+'))
}

type TileParts = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

/// `GET /replays/{id}/{revision}/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png`.
pub async fn get_tile(
    State(state): State<AppState>,
    UrlPath((id, revision, layer, palette, frame, z, x, ypng)): UrlPath<TileParts>,
) -> Response {
    let Some(y) = ypng.strip_suffix(".png") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let zoom_ok = z.parse::<u32>().is_ok_and(|z| CASE_ZOOMS.contains(&z)) && digits(&z);
    let known_layer = case::layer(&layer).is_some();
    if palette != CASE_PALETTE
        || !known_layer
        || !frame_path_ok(&frame)
        || !zoom_ok
        || !digits(&x)
        || !digits(y)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let key = (id.clone(), revision.clone());
    let is_ready = match state.replay_ready.get(&key) {
        Some(v) => v,
        None => {
            let v = match ready(&state, id.clone(), revision.clone()).await {
                Ok(r) => r.is_some(),
                Err(resp) => return resp,
            };
            state.replay_ready.insert(key, v);
            v
        }
    };
    if !is_ready {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = radar_replay::quota::replay_root(&state.config.tile_dir)
        .join(&id)
        .join(&revision)
        .join("tiles")
        .join(&layer)
        .join(&palette)
        .join(&frame)
        .join(&z)
        .join(&x)
        .join(format!("{y}.png"));
    let read = tokio::task::spawn_blocking(move || read_capped(&path, MAX_TILE_BYTES)).await;
    match read {
        Ok(Some(bytes)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, TILE_CC),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_names_follow_the_importer_rule() {
        for ok in ["contract-1", "r1", "2026.11_a"] {
            assert!(valid_revision(ok), "{ok}");
        }
        for bad in ["", ".", "..", ".staging-r1", "Up", "a/b", "a\\b", "-x"] {
            assert!(!valid_revision(bad), "{bad}");
        }
    }

    #[test]
    fn replay_root_is_private_under_the_tile_lane() {
        for hidden in [
            "/replays/a/b/revision.json",
            "/./replays/x",
            "/%72eplays/x",
            "//Replays/x",
        ] {
            assert!(is_private_tile_path(hidden), "{hidden}");
        }
        for open in ["/radar/classic/t/4/1/2.png", "/packed/replays/x"] {
            assert!(!is_private_tile_path(open), "{open}");
        }
    }

    #[test]
    fn frame_paths_allow_only_timestamp_characters() {
        assert!(frame_path_ok("2024-05-21T19:00:17+00:00"));
        assert!(!frame_path_ok(".."));
        assert!(!frame_path_ok("2024/05"));
        assert!(!frame_path_ok(""));
    }
}
