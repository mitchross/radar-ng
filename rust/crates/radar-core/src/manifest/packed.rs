//! `layers.<layer>.packed`: packed data tile metadata (`docs/packed-tiles.md`, "Manifest").
//!
//! The writers take `manifest.lock` like every other manifest writer. They change only the
//! `packed` key of an existing layer: a layer that is not in the manifest is left absent. Call them
//! after the bucket file is written, and remove buckets here before their directories are deleted.

use chrono::{DateTime, Duration, NaiveDateTime, Timelike, Utc};
use serde_json::{Map, Value, json};
use std::path::Path;

use super::{Manifest, ManifestError, with_locked_manifest};
use crate::time::parse_iso_as_utc;

/// Layer key that holds the packed metadata.
pub const PACKED_KEY: &str = "packed";
/// Packed format version in the manifest object.
pub const PACKED_VERSION: i64 = 1;
/// Version of a layer with typed codes. v1 clients check the version and fall back to rasters.
pub const PACKED_TYPED_VERSION: i64 = 2;
/// Tile URL template of packed buckets.
pub const PACKED_URL_TEMPLATE: &str = "/tiles/packed/{layer}/{bucket}/{z}/{x}/{y}.png?v={v}";
/// Length of an observation bucket window.
pub const OBSERVATION_BUCKET_MINUTES: i64 = 30;
/// Directory under `TILE_DIR` that holds the packed layers.
pub const PACKED_DIR: &str = "packed";

/// The fixed part of a layer's `packed` object.
#[derive(Debug, Clone, PartialEq)]
pub struct PackedLayout {
    /// Atlas block edge in pixels.
    pub tile_size: u32,
    /// Lowest packed zoom (512-px zoom).
    pub min_zoom: u32,
    /// Highest packed zoom (512-px zoom). Clients overzoom past it.
    pub max_zoom: u32,
    /// Blocks per atlas row.
    pub cols: u32,
    /// Most slots per bucket.
    pub max_slots: u32,
    /// Palette key the values belong to.
    pub color_key: String,
    /// Categorical layers are not packed in v1; always `false` today.
    pub categorical: bool,
    /// Value step per code.
    pub scale: f64,
    /// Value at code 0.
    pub offset: f64,
    /// Typed codes (`class * stride + q`), or `None` for plain `q` codes.
    pub types: Option<PackedTypes>,
}

/// `packed.types`: the code space of a layer colored by precipitation type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedTypes {
    /// Codes per class.
    pub stride: u32,
    /// Class names in class-index order.
    pub classes: Vec<String>,
}

/// One entry of `packed.buckets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedBucket {
    /// Bucket directory name.
    pub id: String,
    /// Bucket version from its `.packed-bucket-v1.json`.
    pub v: u64,
    /// Slot timestamps, the same strings as the layer's `frames[].timestamp`.
    pub slots: Vec<String>,
}

impl PackedBucket {
    fn to_value(&self) -> Value {
        json!({"id": self.id, "v": self.v, "slots": self.slots})
    }

    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_string(),
            v: value.get("v")?.as_u64()?,
            slots: value
                .get("slots")?
                .as_array()?
                .iter()
                .map(|s| s.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()?,
        })
    }
}

fn packed_value(layout: &PackedLayout, buckets: &[PackedBucket]) -> Value {
    let version = if layout.types.is_some() {
        PACKED_TYPED_VERSION
    } else {
        PACKED_VERSION
    };
    let mut value = json!({
        "version": version,
        "tile_size": layout.tile_size,
        "min_zoom": layout.min_zoom,
        "max_zoom": layout.max_zoom,
        "cols": layout.cols,
        "max_slots": layout.max_slots,
        "color_key": layout.color_key,
        "categorical": layout.categorical,
        "scale": layout.scale,
        "offset": layout.offset,
        "url_template": PACKED_URL_TEMPLATE,
        "buckets": buckets.iter().map(PackedBucket::to_value).collect::<Vec<_>>(),
    });
    if let Some(types) = &layout.types {
        value["types"] = json!({"stride": types.stride, "classes": types.classes});
    }
    value
}

/// The `packed` object for these buckets (sorted by id), or `None` when there are none. Run layers
/// put it in `replace_layer`'s metadata, so frames and buckets change in one manifest write.
pub fn packed_metadata(layout: &PackedLayout, mut buckets: Vec<PackedBucket>) -> Option<Value> {
    if buckets.is_empty() {
        return None;
    }
    buckets.sort_by(|a, b| a.id.cmp(&b.id));
    Some(packed_value(layout, &buckets))
}

/// Run buckets as a run layer lists them. A slot keeps its timestamp only when the layer shows
/// that frame from the bucket's run (`frames` are `(timestamp, run)`); any other slot becomes `""`
/// in place, so slots keep their positions and each frame is in at most one bucket. Buckets with
/// no shown slot are dropped.
pub fn run_layer_buckets<'a>(
    mut buckets: Vec<PackedBucket>,
    frames: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<PackedBucket> {
    let mut shown: std::collections::BTreeSet<(String, String)> = frames
        .into_iter()
        .map(|(ts, run)| (canonical_or_raw(ts), run.to_string()))
        .collect();
    buckets.sort_by(|a, b| a.id.cmp(&b.id));
    buckets
        .into_iter()
        .filter_map(|mut bucket| {
            let run = run_of_bucket(&bucket.id).to_string();
            for slot in &mut bucket.slots {
                // Taking the key out blanks any later copy of the same frame.
                if !shown.remove(&(canonical_or_raw(slot), run.clone())) {
                    slot.clear();
                }
            }
            bucket.slots.iter().any(|s| !s.is_empty()).then_some(bucket)
        })
        .collect()
}

fn canonical_or_raw(timestamp: &str) -> String {
    super::canonical_timestamp(timestamp).unwrap_or_else(|_| timestamp.to_string())
}

/// The buckets listed under `layers.<layer>.packed`. Malformed entries are skipped.
pub fn packed_buckets(manifest: &Manifest, layer: &str) -> Vec<PackedBucket> {
    manifest
        .layers
        .get(layer)
        .map(|l| buckets_of(&l.extra))
        .unwrap_or_default()
}

fn buckets_of(extra: &Map<String, Value>) -> Vec<PackedBucket> {
    extra
        .get(PACKED_KEY)
        .and_then(|p| p.get("buckets"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(PackedBucket::from_value).collect())
        .unwrap_or_default()
}

/// Write `packed` with these buckets (sorted by id), or remove it when there are none.
fn store(extra: &mut Map<String, Value>, layout: &PackedLayout, mut buckets: Vec<PackedBucket>) {
    if buckets.is_empty() {
        extra.remove(PACKED_KEY);
        return;
    }
    buckets.sort_by(|a, b| a.id.cmp(&b.id));
    extra.insert(PACKED_KEY.to_string(), packed_value(layout, &buckets));
}

/// The layout already in the manifest, for writers that only drop buckets.
fn stored_layout(extra: &Map<String, Value>) -> Option<PackedLayout> {
    let p = extra.get(PACKED_KEY)?;
    let u = |k: &str| p.get(k).and_then(Value::as_u64).map(|v| v as u32);
    Some(PackedLayout {
        tile_size: u("tile_size")?,
        min_zoom: u("min_zoom")?,
        max_zoom: u("max_zoom")?,
        cols: u("cols")?,
        max_slots: u("max_slots")?,
        color_key: p.get("color_key")?.as_str()?.to_string(),
        categorical: p.get("categorical")?.as_bool()?,
        scale: p.get("scale")?.as_f64()?,
        offset: p.get("offset")?.as_f64()?,
        types: p.get("types").and_then(|t| {
            Some(PackedTypes {
                stride: t.get("stride")?.as_u64()? as u32,
                classes: t
                    .get("classes")?
                    .as_array()?
                    .iter()
                    .map(|c| c.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()?,
            })
        }),
    })
}

/// Add or update one bucket of `layer`. An existing entry with a higher `v` is kept (a late,
/// older writer never rolls a bucket back). When `layout` differs from the stored one, the other
/// buckets are dropped: their codes belong to the old layout. No-op when the layer is not in the
/// manifest.
pub fn upsert_packed_bucket(
    state_dir: &Path,
    layer: &str,
    layout: &PackedLayout,
    bucket: &PackedBucket,
) -> Result<Manifest, ManifestError> {
    with_locked_manifest(state_dir, Utc::now(), |manifest| {
        let Some(entry) = manifest.layers.get_mut(layer) else {
            return Ok(());
        };
        let mut buckets = buckets_of(&entry.extra);
        if stored_layout(&entry.extra).is_some_and(|stored| stored != *layout) {
            buckets.retain(|b| b.id == bucket.id);
        }
        match buckets.iter_mut().find(|b| b.id == bucket.id) {
            Some(existing) if existing.v > bucket.v => {}
            Some(existing) => *existing = bucket.clone(),
            None => buckets.push(bucket.clone()),
        }
        store(&mut entry.extra, layout, buckets);
        Ok(())
    })
}

/// Replace every bucket of `layer` (run layers list only the current run). An empty list removes
/// `packed`. No-op when the layer is not in the manifest.
pub fn replace_packed_buckets(
    state_dir: &Path,
    layer: &str,
    layout: &PackedLayout,
    buckets: Vec<PackedBucket>,
) -> Result<Manifest, ManifestError> {
    with_locked_manifest(state_dir, Utc::now(), |manifest| {
        if let Some(entry) = manifest.layers.get_mut(layer) {
            store(&mut entry.extra, layout, buckets);
        }
        Ok(())
    })
}

/// Drop every bucket of `layer` whose id matches `expired`, and return the dropped ids. When none
/// is left, `packed` is removed. Run this before deleting the bucket directories.
pub fn remove_packed_buckets(
    state_dir: &Path,
    layer: &str,
    expired: impl Fn(&str) -> bool,
) -> Result<Vec<String>, ManifestError> {
    let mut dropped = Vec::new();
    with_locked_manifest(state_dir, Utc::now(), |manifest| {
        let Some(entry) = manifest.layers.get_mut(layer) else {
            return Ok(());
        };
        let Some(layout) = stored_layout(&entry.extra) else {
            return Ok(());
        };
        let (gone, kept): (Vec<_>, Vec<_>) = buckets_of(&entry.extra)
            .into_iter()
            .partition(|b| expired(&b.id));
        dropped = gone.into_iter().map(|b| b.id).collect();
        store(&mut entry.extra, &layout, kept);
        Ok(())
    })?;
    Ok(dropped)
}

/// Observation bucket of a frame timestamp: the 30-minute UTC window start, `YYYY-MM-DDTHH:MM:00Z`.
pub fn observation_bucket_id(timestamp: &str) -> Option<String> {
    let t = parse_iso_as_utc(timestamp)?;
    let minute = t.minute() - t.minute() % OBSERVATION_BUCKET_MINUTES as u32;
    let start = t.with_minute(minute)?.with_second(0)?.with_nanosecond(0)?;
    Some(start.format("%Y-%m-%dT%H:%M:00Z").to_string())
}

/// Start of an observation bucket window, parsed from its id.
pub fn observation_bucket_start(id: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(id, "%Y-%m-%dT%H:%M:%SZ")
        .ok()
        .map(|n| n.and_utc())
}

/// `true` when the whole window of observation bucket `id` is older than `cutoff`. Ids that do not
/// parse are never expired.
pub fn observation_bucket_expired(id: &str, cutoff: DateTime<Utc>) -> bool {
    observation_bucket_start(id)
        .is_some_and(|start| start + Duration::minutes(OBSERVATION_BUCKET_MINUTES) <= cutoff)
}

/// Bucket id of part `part` of run `run`: the run itself for part 0, then `<run>.p<k>`.
pub fn run_bucket_id(run: &str, part: usize) -> String {
    if part == 0 {
        run.to_string()
    } else {
        format!("{run}.p{part}")
    }
}

/// The run a run bucket id belongs to (`<run>` or `<run>.p<k>`).
pub fn run_of_bucket(id: &str) -> &str {
    match id.rsplit_once(".p") {
        Some((run, k)) if !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) => run,
        _ => id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{AddOptions, add_frame, read_manifest, remove_frame};

    fn layout() -> PackedLayout {
        PackedLayout {
            tile_size: 512,
            min_zoom: 3,
            max_zoom: 6,
            cols: 2,
            max_slots: 24,
            color_key: "reflectivity".into(),
            categorical: false,
            scale: 0.5,
            offset: -32.0,
            types: None,
        }
    }

    fn bucket(id: &str, v: u64, slots: &[&str]) -> PackedBucket {
        PackedBucket {
            id: id.into(),
            v,
            slots: slots.iter().map(|s| s.to_string()).collect(),
        }
    }

    const T0: &str = "2026-10-15T18:30:38+00:00";
    const T1: &str = "2026-10-15T18:32:40+00:00";

    #[test]
    fn bucket_ids_are_window_starts_and_run_parts() {
        assert_eq!(
            observation_bucket_id(T0).as_deref(),
            Some("2026-10-15T18:30:00Z")
        );
        assert_eq!(
            observation_bucket_id("2026-10-15T18:29:59.900000+00:00").as_deref(),
            Some("2026-10-15T18:00:00Z")
        );
        assert_eq!(
            observation_bucket_id("2026-10-15T14:45:00-04:00").as_deref(),
            Some("2026-10-15T18:30:00Z")
        );
        assert_eq!(observation_bucket_id("nope"), None);
        let start = observation_bucket_start("2026-10-15T18:30:00Z").unwrap();
        assert!(!observation_bucket_expired(
            "2026-10-15T18:30:00Z",
            start + Duration::minutes(29)
        ));
        assert!(observation_bucket_expired(
            "2026-10-15T18:30:00Z",
            start + Duration::minutes(30)
        ));
        assert!(!observation_bucket_expired("garbage", start));
        let run = "2026-10-15T18:30:00+00:00";
        assert_eq!(run_bucket_id(run, 0), run);
        assert_eq!(run_bucket_id(run, 2), format!("{run}.p2"));
        assert_eq!(run_of_bucket(&run_bucket_id(run, 2)), run);
        assert_eq!(run_of_bucket(run), run);
    }

    #[test]
    fn run_layer_buckets_blank_frames_from_other_runs_and_publish_with_the_frames() {
        let (a, b, c) = (
            "2026-10-15T13:00:00+00:00",
            "2026-10-15T14:00:00+00:00",
            "2026-10-15T15:00:00+00:00",
        );
        let old = "20261015_12";
        let new = "20261015_13";
        let buckets = vec![
            bucket(old, 3, &[a, b, c]),
            bucket(&run_bucket_id(old, 1), 1, &[a]),
            bucket(new, 2, &[c, b]),
        ];
        // The new run shows 14z and 15z; 13z is the old run's (the spelling may differ).
        let frames = [("2026-10-15T13:00:00Z", old), (b, new), (c, new)];
        let listed = run_layer_buckets(buckets, frames);
        assert_eq!(
            listed,
            vec![bucket(old, 3, &[a, "", ""]), bucket(new, 2, &[c, b])],
            "a copy in a later part and a bucket with nothing shown are dropped"
        );

        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        assert_eq!(packed_metadata(&layout(), Vec::new()), None);
        let mut metadata = Map::new();
        metadata.insert(
            PACKED_KEY.into(),
            packed_metadata(&layout(), listed.clone()).unwrap(),
        );
        let options = super::super::ReplaceOptions {
            palettes: vec!["classic".into()],
            frames: Vec::new(),
            layer_metadata: Some(metadata),
        };
        let m = super::super::replace_layer(
            state,
            "temperature",
            &[a.to_string(), b.to_string(), c.to_string()],
            &options,
        )
        .unwrap();
        assert_eq!(packed_buckets(&m, "temperature"), listed);
        assert_eq!(m.layers["temperature"].frames.len(), 3);
    }

    #[test]
    fn packed_metadata_upsert_and_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        // No layer yet: nothing is created.
        upsert_packed_bucket(state, "radar", &layout(), &bucket("b", 1, &[T0])).unwrap();
        assert!(!read_manifest(state).unwrap().layers.contains_key("radar"));

        add_frame(state, "radar", T0, &AddOptions::default()).unwrap();
        add_frame(state, "radar", T1, &AddOptions::default()).unwrap();
        let old = "2026-10-15T14:00:00Z";
        let cur = "2026-10-15T18:30:00Z";
        upsert_packed_bucket(state, "radar", &layout(), &bucket(cur, 1, &[T0])).unwrap();
        upsert_packed_bucket(state, "radar", &layout(), &bucket(old, 9, &["x"])).unwrap();
        upsert_packed_bucket(state, "radar", &layout(), &bucket(cur, 2, &[T0, T1])).unwrap();
        // A late writer with an older version does not roll the bucket back.
        let m = upsert_packed_bucket(state, "radar", &layout(), &bucket(cur, 1, &[T0])).unwrap();
        assert_eq!(
            packed_buckets(&m, "radar"),
            vec![bucket(old, 9, &["x"]), bucket(cur, 2, &[T0, T1])]
        );

        // The exact JSON clients read.
        let text = std::fs::read_to_string(state.join("manifest.json")).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v["layers"]["radar"]["packed"],
            json!({
                "version": 1, "tile_size": 512, "min_zoom": 3, "max_zoom": 6, "cols": 2,
                "max_slots": 24, "color_key": "reflectivity", "categorical": false,
                "scale": 0.5, "offset": -32.0,
                "url_template": "/tiles/packed/{layer}/{bucket}/{z}/{x}/{y}.png?v={v}",
                "buckets": [
                    {"id": old, "v": 9, "slots": ["x"]},
                    {"id": cur, "v": 2, "slots": [T0, T1]},
                ],
            })
        );
        assert_eq!(v["schema_version"], 2);
        // Frame writes keep the packed key.
        let m = add_frame(state, "radar", T1, &AddOptions::default()).unwrap();
        assert_eq!(packed_buckets(&m, "radar").len(), 2);

        // Expiry drops matching buckets; the last one removes the key.
        let cutoff = observation_bucket_start(cur).unwrap();
        let dropped =
            remove_packed_buckets(state, "radar", |id| observation_bucket_expired(id, cutoff))
                .unwrap();
        assert_eq!(dropped, vec![old.to_string()]);
        let m = read_manifest(state).unwrap();
        assert_eq!(packed_buckets(&m, "radar"), vec![bucket(cur, 2, &[T0, T1])]);
        remove_packed_buckets(state, "radar", |_| true).unwrap();
        let m = read_manifest(state).unwrap();
        assert!(!m.layers["radar"].extra.contains_key(PACKED_KEY));

        // Replace sets the run buckets; an empty list removes the key; removing every frame
        // removes the layer with its metadata.
        let run = vec![bucket("r", 1, &[T1])];
        let m = replace_packed_buckets(state, "radar", &layout(), run.clone()).unwrap();
        assert_eq!(packed_buckets(&m, "radar"), run);
        let m = replace_packed_buckets(state, "radar", &layout(), Vec::new()).unwrap();
        assert!(!m.layers["radar"].extra.contains_key(PACKED_KEY));
        remove_frame(state, "radar", T0).unwrap();
        let m = remove_frame(state, "radar", T1).unwrap();
        assert!(!m.layers.contains_key("radar"));
    }
    #[test]
    fn typed_layout_is_version_2_and_drops_buckets_of_the_old_layout() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        add_frame(state, "radar", T0, &AddOptions::default()).unwrap();
        let old = "2026-10-15T18:00:00Z";
        let cur = "2026-10-15T18:30:00Z";
        upsert_packed_bucket(state, "radar", &layout(), &bucket(old, 3, &["x"])).unwrap();
        let typed = PackedLayout {
            types: Some(PackedTypes {
                stride: 32,
                classes: vec!["rain".into(), "snow".into(), "mix".into()],
            }),
            ..layout()
        };
        let m = upsert_packed_bucket(state, "radar", &typed, &bucket(cur, 1, &[T0])).unwrap();
        assert_eq!(packed_buckets(&m, "radar"), vec![bucket(cur, 1, &[T0])]);
        let packed = &m.layers["radar"].extra[PACKED_KEY];
        assert_eq!(packed["version"], 2);
        assert_eq!(
            packed["types"],
            json!({"stride": 32, "classes": ["rain", "snow", "mix"]})
        );
        assert_eq!(stored_layout(&m.layers["radar"].extra), Some(typed.clone()));
        // The same layout keeps its buckets.
        let m = upsert_packed_bucket(state, "radar", &typed, &bucket(old, 1, &[T0])).unwrap();
        assert_eq!(packed_buckets(&m, "radar").len(), 2);
    }
}
