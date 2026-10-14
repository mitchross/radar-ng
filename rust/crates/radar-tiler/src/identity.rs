//! Render identity: canonical JSON, digests, and the publication identity document
//! (spec-tiler section 10).
//!
//! Canonical JSON follows Python's `json.dumps(sort_keys=True, separators=(",", ":"))`
//! with `ensure_ascii=True`, and floats use Python `repr` (shortest round-trip digits, with
//! exponent form outside `1e-4 <= |x| < 1e16`). `int 1` and `float 1.0` therefore hash
//! differently, as in Python (spec H4).
//!
//! Rust publishes under its own renderer name ([`RENDERER`], spec option B). The digest
//! scheme is kept identical to Python so the golden vectors in spec section 18 still hold,
//! but the identity never claims Python's `indexed` renderer.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use blake2::digest::consts::U16;
use blake2::{Blake2b, Digest as _};
use serde_json::Value;
use sha2::Sha256;

use crate::classify::{ClassModel, build_categorical_class_model, build_class_model};
use crate::error::{Result, TilerError};
use crate::palettes::ColorTable;

/// Renderer name written into every identity produced by this crate.
pub const RENDERER: &str = "indexed-rs1";
/// Algorithm for continuous frames (bilinear physical sampling, then classify).
pub const ALGORITHM_CONTINUOUS: &str = "physical-bilinear-classify-v1";
/// Algorithm for categorical frames (nearest class sampling).
pub const ALGORITHM_CATEGORICAL: &str = "category-nearest-v1";
/// Marker schema version written by this crate (spec section 11).
pub const MARKER_SCHEMA: i64 = 2;

// ---------------------------------------------------------------------------
// Canonical JSON
// ---------------------------------------------------------------------------

/// A value in the canonical-JSON model of spec section 10.1.
#[derive(Debug, Clone, PartialEq)]
pub enum Canon {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// Python `int`.
    Int(i64),
    /// Python `float` (non-finite values render as `{"float": "nan"}` and friends).
    Float(f64),
    /// Python `str`.
    Str(String),
    /// Python `bytes`, rendered as `{"bytes": "<hex>"}`.
    Bytes(Vec<u8>),
    /// JSON array.
    Array(Vec<Canon>),
    /// JSON object; keys are sorted when rendered.
    Object(Vec<(String, Canon)>),
    /// A NumPy array, rendered as `{"array_digest", "dtype", "shape"}`.
    NdArray {
        /// Content digest (see [`array_digest`]).
        digest: String,
        /// NumPy `dtype.str`, for example `<f8` or `|u1`.
        dtype: &'static str,
        /// Array shape.
        shape: Vec<usize>,
    },
}

impl Canon {
    /// Convert a parsed JSON value (integers stay integers, floats stay floats).
    pub fn from_json(v: &Value) -> Canon {
        match v {
            Value::Null => Canon::Null,
            Value::Bool(b) => Canon::Bool(*b),
            Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
                (Some(i), _, _) => Canon::Int(i),
                (None, Some(u), _) if i64::try_from(u).is_err() => Canon::Float(u as f64),
                (None, Some(u), _) => Canon::Int(u as i64),
                (None, None, Some(f)) => Canon::Float(f),
                _ => Canon::Null,
            },
            Value::String(s) => Canon::Str(s.clone()),
            Value::Array(items) => Canon::Array(items.iter().map(Canon::from_json).collect()),
            Value::Object(map) => Canon::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), Canon::from_json(v)))
                    .collect(),
            ),
        }
    }

    /// Render as canonical JSON text.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        write_canon(self, &mut out);
        out
    }
}

fn write_canon(c: &Canon, out: &mut String) {
    match c {
        Canon::Null => out.push_str("null"),
        Canon::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Canon::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Canon::Float(f) => {
            if f.is_finite() {
                out.push_str(&python_float_repr(*f));
            } else {
                let s = if f.is_nan() {
                    "nan"
                } else if *f > 0.0 {
                    "inf"
                } else {
                    "-inf"
                };
                let _ = write!(out, "{{\"float\":\"{s}\"}}");
            }
        }
        Canon::Str(s) => write_json_string(s, out),
        Canon::Bytes(b) => {
            let _ = write!(out, "{{\"bytes\":\"{}\"}}", hex::encode(b));
        }
        Canon::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canon(item, out);
            }
            out.push(']');
        }
        Canon::Object(pairs) => {
            let mut sorted: Vec<&(String, Canon)> = pairs.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            out.push('{');
            for (i, (k, v)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(k, out);
                out.push(':');
                write_canon(v, out);
            }
            out.push('}');
        }
        Canon::NdArray {
            digest,
            dtype,
            shape,
        } => {
            let shape_items: Vec<Canon> = shape.iter().map(|&d| Canon::Int(d as i64)).collect();
            let obj = Canon::Object(vec![
                ("array_digest".into(), Canon::Str(digest.clone())),
                ("dtype".into(), Canon::Str((*dtype).to_string())),
                ("shape".into(), Canon::Array(shape_items)),
            ]);
            write_canon(&obj, out);
        }
    }
}

/// JSON string literal with Python's `ensure_ascii=True` escaping (lowercase `\uXXXX`).
pub fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ' '..='~' => out.push(ch),
            c => {
                let cp = c as u32;
                if cp > 0xFFFF {
                    let v = cp - 0x10000;
                    let _ = write!(
                        out,
                        "\\u{:04x}\\u{:04x}",
                        0xD800 + (v >> 10),
                        0xDC00 + (v & 0x3FF)
                    );
                } else {
                    let _ = write!(out, "\\u{cp:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// Python `repr(float)` for finite values: shortest round-trip digits; exponent form when the
/// decimal exponent is below -4 or at least 16, written as `e`, a sign and at least two digits.
pub fn python_float_repr(x: f64) -> String {
    debug_assert!(x.is_finite());
    // Rust's `{:e}` gives the shortest round-trip digits: "-1.2345e-7", "1e16", "0e0".
    let sci = format!("{x:e}");
    let (sign, body) = match sci.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", sci.as_str()),
    };
    let (mantissa, exp_text) = body.split_once('e').expect("{:e} always has an exponent");
    let exp: i32 = exp_text.parse().expect("exponent is an integer");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let mut out = String::from(sign);
    if !(-4..16).contains(&exp) {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let esign = if exp < 0 { '-' } else { '+' };
        let _ = write!(out, "e{esign}{:02}", exp.unsigned_abs());
    } else if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            out.push_str(&digits);
            out.extend(std::iter::repeat_n('0', int_len - digits.len()));
            out.push_str(".0");
        } else {
            out.push_str(&digits[..int_len]);
            out.push('.');
            out.push_str(&digits[int_len..]);
        }
    } else {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-exp - 1) as usize));
        out.push_str(&digits);
    }
    out
}

/// `_stable_digest`: SHA-256 hex of the canonical JSON of `value`.
pub fn stable_digest(value: &Canon) -> String {
    hex::encode(sha2::Sha256::digest(value.to_json().as_bytes()))
}

/// SHA-256 hex of a byte slice.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Digest of a parsed palette color-key object (the palette payload of spec 10.1).
pub fn palette_digest(table: &ColorTable) -> String {
    stable_digest(&Canon::from_json(table.raw()))
}

// ---------------------------------------------------------------------------
// Array digests
// ---------------------------------------------------------------------------

/// Hash `{"dtype": <dtype>, "shape": [...]}` followed by raw bytes, as `_array_content_digest`.
fn array_hasher(dtype: &str, shape: &[usize]) -> Sha256 {
    use sha2::Digest as _;
    let shape_json: Vec<String> = shape.iter().map(usize::to_string).collect();
    let header = format!(
        "{{\"dtype\":\"{dtype}\",\"shape\":[{}]}}",
        shape_json.join(",")
    );
    let mut h = Sha256::new();
    h.update(header.as_bytes());
    h
}

/// Content digest of an `f32` grid (`<f4`), hashing little-endian bytes in chunks.
pub fn array_digest_f32(values: &[f32], shape: &[usize]) -> String {
    use sha2::Digest as _;
    let mut h = array_hasher("<f4", shape);
    let mut buf = [0u8; 4 * 4096];
    for chunk in values.chunks(4096) {
        let n = chunk.len() * 4;
        for (dst, v) in buf[..n].chunks_exact_mut(4).zip(chunk) {
            dst.copy_from_slice(&v.to_le_bytes());
        }
        h.update(&buf[..n]);
    }
    hex::encode(h.finalize())
}

/// Content digest of an `f64` array (`<f8`).
pub fn array_digest_f64(values: &[f64], shape: &[usize]) -> String {
    use sha2::Digest as _;
    let mut h = array_hasher("<f8", shape);
    let mut buf = [0u8; 8 * 4096];
    for chunk in values.chunks(4096) {
        let n = chunk.len() * 8;
        for (dst, v) in buf[..n].chunks_exact_mut(8).zip(chunk) {
            dst.copy_from_slice(&v.to_le_bytes());
        }
        h.update(&buf[..n]);
    }
    hex::encode(h.finalize())
}

/// Content digest of a `u8` array (`|u1`).
pub fn array_digest_u8(values: &[u8], shape: &[usize]) -> String {
    use sha2::Digest as _;
    let mut h = array_hasher("|u1", shape);
    h.update(values);
    hex::encode(h.finalize())
}

/// Fast-changing coordinate fingerprint: shape, dtype and BLAKE2b-128 of the raw bytes
/// (`_array_fingerprint`). Used only as a cache key and as part of the grid digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    /// Array shape.
    pub shape: Vec<usize>,
    /// NumPy `dtype.str`.
    pub dtype: &'static str,
    /// BLAKE2b with 16-byte output over the little-endian bytes.
    pub digest: [u8; 16],
}

/// Fingerprint a 1-D `f64` coordinate axis.
pub fn fingerprint_f64(values: &[f64]) -> Fingerprint {
    type Blake2b128 = Blake2b<U16>;
    let mut h = Blake2b128::new();
    let mut buf = [0u8; 8 * 4096];
    for chunk in values.chunks(4096) {
        let n = chunk.len() * 8;
        for (dst, v) in buf[..n].chunks_exact_mut(8).zip(chunk) {
            dst.copy_from_slice(&v.to_le_bytes());
        }
        h.update(&buf[..n]);
    }
    let mut digest = [0u8; 16];
    digest.copy_from_slice(&h.finalize());
    Fingerprint {
        shape: vec![values.len()],
        dtype: "<f8",
        digest,
    }
}

fn fingerprint_canon(fp: &Fingerprint) -> Canon {
    Canon::Array(vec![
        Canon::Array(fp.shape.iter().map(|&d| Canon::Int(d as i64)).collect()),
        Canon::Str(fp.dtype.to_string()),
        Canon::Bytes(fp.digest.to_vec()),
    ])
}

// ---------------------------------------------------------------------------
// Grid identity
// ---------------------------------------------------------------------------

/// Coordinate identity of a source grid (spec 10.1, `_grid_spec_digest`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoordinateIdentity {
    /// Regular grid: fingerprints of the full 1-D lat and lon axes.
    Regular {
        /// Latitude axis.
        lats: Fingerprint,
        /// Longitude axis.
        lons: Fingerprint,
    },
    /// Projected grid: CRS identity string and fingerprints of the native x and y axes.
    Projected {
        /// CRS identity (`Crs::identity_string`).
        crs: String,
        /// Native x axis.
        x: Fingerprint,
        /// Native y axis.
        y: Fingerprint,
    },
}

/// Everything about a source grid that affects tile coordinate mapping.
///
/// Floating-point bounds are kept as `f64` for the digest but compared and hashed by bit
/// pattern (with `-0.0` normalised to `0.0`) so the value can key the geometry cache.
#[derive(Debug, Clone)]
pub struct GridIdentity {
    /// `"regular"` or `"projected"`.
    pub kind: &'static str,
    /// Grid height (rows).
    pub height: usize,
    /// Grid width (columns).
    pub width: usize,
    /// Finite latitude bounds `(min, max)`.
    pub lat_bounds: (f64, f64),
    /// Finite longitude bounds `(min, max)`.
    pub lon_bounds: (f64, f64),
    /// Coordinate identity.
    pub coordinates: CoordinateIdentity,
}

fn bits(v: f64) -> u64 {
    if v == 0.0 { 0 } else { v.to_bits() }
}

impl GridIdentity {
    fn key(&self) -> (&'static str, usize, usize, [u64; 4], &CoordinateIdentity) {
        (
            self.kind,
            self.height,
            self.width,
            [
                bits(self.lat_bounds.0),
                bits(self.lat_bounds.1),
                bits(self.lon_bounds.0),
                bits(self.lon_bounds.1),
            ],
            &self.coordinates,
        )
    }
}

impl PartialEq for GridIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for GridIdentity {}

impl std::hash::Hash for GridIdentity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// `_grid_spec_digest`. The sampling mode is deliberately excluded (spec 10.1).
pub fn grid_spec_digest(g: &GridIdentity) -> String {
    let coords = match &g.coordinates {
        CoordinateIdentity::Regular { lats, lons } => {
            Canon::Array(vec![fingerprint_canon(lats), fingerprint_canon(lons)])
        }
        CoordinateIdentity::Projected { crs, x, y } => Canon::Array(vec![
            Canon::Str(crs.clone()),
            fingerprint_canon(x),
            fingerprint_canon(y),
        ]),
    };
    stable_digest(&Canon::Object(vec![
        ("kind".into(), Canon::Str(g.kind.to_string())),
        ("height".into(), Canon::Int(g.height as i64)),
        ("width".into(), Canon::Int(g.width as i64)),
        (
            "lat_bounds".into(),
            Canon::Array(vec![
                Canon::Float(g.lat_bounds.0),
                Canon::Float(g.lat_bounds.1),
            ]),
        ),
        (
            "lon_bounds".into(),
            Canon::Array(vec![
                Canon::Float(g.lon_bounds.0),
                Canon::Float(g.lon_bounds.1),
            ]),
        ),
        ("coordinate_identity".into(), coords),
    ]))
}

/// `_stable_digest({"tile_size": T, "zoom_levels": [z...]})`. Zoom order matters.
pub fn tile_spec_digest(tile_size: usize, zoom_levels: &[u32]) -> String {
    stable_digest(&Canon::Object(vec![
        ("tile_size".into(), Canon::Int(tile_size as i64)),
        (
            "zoom_levels".into(),
            Canon::Array(
                zoom_levels
                    .iter()
                    .map(|&z| Canon::Int(i64::from(z)))
                    .collect(),
            ),
        ),
    ]))
}

// ---------------------------------------------------------------------------
// Pyramid identity
// ---------------------------------------------------------------------------

/// Field names of [`PyramidIdentity`], in sorted order (spec 10.4).
pub const IDENTITY_FIELDS: [&str; 10] = [
    "algorithm",
    "grid_spec_digest",
    "palette_digest",
    "palette_name",
    "policy_digest",
    "renderer",
    "semantic_digest",
    "source_digest",
    "source_id",
    "tile_spec_digest",
];

/// Inputs that make an immutable palette pyramid reproducible (spec 10.1, 10.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyramidIdentity {
    /// Renderer name (`indexed-rs1` for this crate).
    pub renderer: String,
    /// Algorithm name.
    pub algorithm: String,
    /// Stable frame identifier supplied by the caller.
    pub source_id: String,
    /// Digest of the full source array.
    pub source_digest: String,
    /// Digest of the grid's coordinate identity.
    pub grid_spec_digest: String,
    /// Palette name.
    pub palette_name: String,
    /// Digest of the palette's color-key object.
    pub palette_digest: String,
    /// Digest of tile size and zoom list.
    pub tile_spec_digest: String,
    /// Digest of the semantic policy (what the pixels mean).
    pub semantic_digest: String,
    /// Digest of the render policy (how the pixels were produced).
    pub policy_digest: String,
}

impl PyramidIdentity {
    fn field_values(&self) -> [(&'static str, &str); 10] {
        [
            ("algorithm", &self.algorithm),
            ("grid_spec_digest", &self.grid_spec_digest),
            ("palette_digest", &self.palette_digest),
            ("palette_name", &self.palette_name),
            ("policy_digest", &self.policy_digest),
            ("renderer", &self.renderer),
            ("semantic_digest", &self.semantic_digest),
            ("source_digest", &self.source_digest),
            ("source_id", &self.source_id),
            ("tile_spec_digest", &self.tile_spec_digest),
        ]
    }

    /// Canonical object form used in the completion marker.
    pub fn to_canon(&self) -> Canon {
        Canon::Object(
            self.field_values()
                .iter()
                .map(|(k, v)| ((*k).to_string(), Canon::Str((*v).to_string())))
                .collect(),
        )
    }

    /// `to_document`: the 10 fields as a map.
    pub fn to_document(&self) -> BTreeMap<&'static str, String> {
        self.field_values()
            .iter()
            .map(|(k, v)| (*k, (*v).to_string()))
            .collect()
    }

    /// `from_document`: exactly the 10 fields, each a non-empty string.
    pub fn from_document(doc: &Value) -> Result<PyramidIdentity> {
        let obj = doc
            .as_object()
            .ok_or_else(|| TilerError::Corrupt("invalid pyramid identity fields".into()))?;
        if obj.len() != IDENTITY_FIELDS.len()
            || IDENTITY_FIELDS.iter().any(|f| !obj.contains_key(*f))
        {
            return Err(TilerError::Corrupt(
                "invalid pyramid identity fields".into(),
            ));
        }
        let get = |name: &str| -> Result<String> {
            match obj.get(name).and_then(Value::as_str) {
                Some(s) if !s.is_empty() => Ok(s.to_string()),
                _ => Err(TilerError::Corrupt(
                    "pyramid identity values must be non-empty strings".into(),
                )),
            }
        };
        Ok(PyramidIdentity {
            renderer: get("renderer")?,
            algorithm: get("algorithm")?,
            source_id: get("source_id")?,
            source_digest: get("source_digest")?,
            grid_spec_digest: get("grid_spec_digest")?,
            palette_name: get("palette_name")?,
            palette_digest: get("palette_digest")?,
            tile_spec_digest: get("tile_spec_digest")?,
            semantic_digest: get("semantic_digest")?,
            policy_digest: get("policy_digest")?,
        })
    }

    /// `is_renderer_compatible_with` (spec 10.3): a known foreign renderer that won the same
    /// logical frame. Crate publication does not use this (the collision rule accepts any
    /// complete marker); it is kept for parity and for the resume check.
    pub fn is_renderer_compatible_with(&self, expected: &PyramidIdentity) -> bool {
        const LEGACY: (&str, &str) = ("legacy", "rgba-bilinear-v1");
        let pair_a = (self.renderer.as_str(), self.algorithm.as_str());
        let pair_b = (expected.renderer.as_str(), expected.algorithm.as_str());
        let set_ok = [
            [LEGACY, ("indexed", "physical-bilinear-classify-v1")],
            [LEGACY, ("indexed", "category-nearest-v1")],
        ]
        .iter()
        .any(|allowed| {
            (pair_a == allowed[0] && pair_b == allowed[1])
                || (pair_a == allowed[1] && pair_b == allowed[0])
        });
        self.renderer != expected.renderer
            && set_ok
            && self.source_id == expected.source_id
            && self.source_digest == expected.source_digest
            && self.grid_spec_digest == expected.grid_spec_digest
            && self.palette_name == expected.palette_name
            && self.palette_digest == expected.palette_digest
            && self.tile_spec_digest == expected.tile_spec_digest
            && self.semantic_digest == expected.semantic_digest
    }
}

// ---------------------------------------------------------------------------
// Frame contract
// ---------------------------------------------------------------------------

/// What the pixels of a frame mean.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameKind {
    /// Physical values, bilinear sampling, then classify.
    Continuous {
        /// Value treated as missing (compared after cast to f32); `None` disables it.
        nodata_value: Option<f64>,
        /// Minimum fraction of non-zero bilinear weight that must be valid (`(0, 1]`).
        min_valid_weight: f64,
    },
    /// Integer category ids, nearest sampling.
    Categorical {
        /// Category id to name.
        category_map: BTreeMap<i64, String>,
    },
}

/// Algorithm, semantic and policy documents for one frame configuration (spec 10.2).
#[derive(Debug, Clone)]
pub struct FrameContract {
    /// Algorithm name.
    pub algorithm: &'static str,
    /// Semantic policy document.
    pub semantic: Canon,
    /// Render policy document.
    pub policy: Canon,
}

/// Build the frame contract for `kind` over `model`.
///
/// `overview` is `Some("max")` only when max-pooled overviews are on; the key is then added
/// to both documents, so existing identities without overviews are unchanged.
pub fn frame_contract(
    kind: &FrameKind,
    model: &ClassModel,
    png_level: u32,
    overview: Option<&str>,
) -> FrameContract {
    let remap = Canon::NdArray {
        digest: array_digest_u8(model.remap(), &[model.remap().len()]),
        dtype: "|u1",
        shape: vec![model.remap().len()],
    };
    let (algorithm, mut semantic, mut policy) = match kind {
        FrameKind::Continuous {
            nodata_value,
            min_valid_weight,
        } => {
            let edges = model.edges().unwrap_or(&[]);
            let nodata = nodata_value.map_or(Canon::Null, Canon::Float);
            let semantic = Canon::Object(vec![
                ("kind".into(), Canon::Str("continuous".into())),
                ("nodata_value".into(), nodata.clone()),
                ("min_valid_weight".into(), Canon::Float(*min_valid_weight)),
            ]);
            let policy = Canon::Object(vec![
                ("sampling".into(), Canon::Str("bilinear".into())),
                ("nodata_value".into(), nodata),
                ("min_valid_weight".into(), Canon::Float(*min_valid_weight)),
                (
                    "model_edges".into(),
                    Canon::NdArray {
                        digest: array_digest_f64(edges, &[edges.len()]),
                        dtype: "<f8",
                        shape: vec![edges.len()],
                    },
                ),
                ("model_remap".into(), remap),
                (
                    "png_compress_level".into(),
                    Canon::Int(i64::from(png_level)),
                ),
            ]);
            (ALGORITHM_CONTINUOUS, semantic, policy)
        }
        FrameKind::Categorical { category_map } => {
            let map = Canon::Object(
                category_map
                    .iter()
                    .map(|(k, v)| (k.to_string(), Canon::Str(v.clone())))
                    .collect(),
            );
            let semantic = Canon::Object(vec![
                ("kind".into(), Canon::Str("categorical".into())),
                ("category_map".into(), map.clone()),
            ]);
            let policy = Canon::Object(vec![
                ("sampling".into(), Canon::Str("nearest".into())),
                ("category_map".into(), map),
                ("model_remap".into(), remap),
                (
                    "png_compress_level".into(),
                    Canon::Int(i64::from(png_level)),
                ),
            ]);
            (ALGORITHM_CATEGORICAL, semantic, policy)
        }
    };
    if let Some(mode) = overview {
        semantic = with_key(semantic, "overview", Canon::Str(mode.into()));
        policy = with_key(policy, "overview", Canon::Str(mode.into()));
    }
    FrameContract {
        algorithm,
        semantic,
        policy,
    }
}

fn with_key(doc: Canon, key: &str, value: Canon) -> Canon {
    match doc {
        Canon::Object(mut pairs) => {
            pairs.push((key.to_string(), value));
            Canon::Object(pairs)
        }
        other => other,
    }
}

/// Build the color tables a frame needs: one [`ClassModel`] over all of them (class numbering
/// depends on the whole palette set, spec H3).
pub fn frame_model(kind: &FrameKind, tables: &[(String, ColorTable)]) -> Result<ClassModel> {
    match kind {
        FrameKind::Continuous { .. } => build_class_model(tables),
        FrameKind::Categorical { category_map } => {
            build_categorical_class_model(tables, category_map)
        }
    }
}

/// Per-palette identities for one frame (spec 10.2 `_publication_identities`).
///
/// `palettes` pairs each palette name with its color table. `source_id` must be non-empty.
pub fn frame_identities(
    source_id: &str,
    source_digest: &str,
    grid_digest: &str,
    zoom_levels: &[u32],
    tile_size: usize,
    contract: &FrameContract,
    palettes: &[(String, ColorTable)],
) -> Result<Vec<(String, PyramidIdentity)>> {
    if source_id.is_empty() {
        return Err(TilerError::Invalid(
            "source_id must be a non-empty stable frame identity".into(),
        ));
    }
    let tile_digest = tile_spec_digest(tile_size, zoom_levels);
    let semantic_digest = stable_digest(&contract.semantic);
    let policy_digest = stable_digest(&contract.policy);
    Ok(palettes
        .iter()
        .map(|(name, table)| {
            (
                name.clone(),
                PyramidIdentity {
                    renderer: RENDERER.to_string(),
                    algorithm: contract.algorithm.to_string(),
                    source_id: source_id.to_string(),
                    source_digest: source_digest.to_string(),
                    grid_spec_digest: grid_digest.to_string(),
                    palette_name: name.clone(),
                    palette_digest: palette_digest(table),
                    tile_spec_digest: tile_digest.clone(),
                    semantic_digest: semantic_digest.clone(),
                    policy_digest: policy_digest.clone(),
                },
            )
        })
        .collect())
}

/// Resume check (spec 10.3 `frame_pyramid_identity_is_compatible`): does a stored identity
/// match the current contract for `palette_name`, given the stored source and grid digests?
#[allow(clippy::too_many_arguments)] // mirrors the Python keyword-argument signature
pub fn frame_identity_is_compatible(
    stored: &PyramidIdentity,
    source_id: &str,
    tables: &[(String, ColorTable)],
    palette_name: &str,
    zoom_levels: &[u32],
    tile_size: usize,
    kind: &FrameKind,
    png_level: u32,
    overview: Option<&str>,
) -> Result<bool> {
    let Some((_, table)) = tables.iter().find(|(n, _)| n == palette_name) else {
        return Ok(false);
    };
    let model = frame_model(kind, tables)?;
    let contract = frame_contract(kind, &model, png_level, overview);
    let expected = frame_identities(
        source_id,
        &stored.source_digest,
        &stored.grid_spec_digest,
        zoom_levels,
        tile_size,
        &contract,
        &[(palette_name.to_string(), table.clone())],
    )?
    .remove(0)
    .1;
    Ok(*stored == expected || stored.is_renderer_compatible_with(&expected))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Golden vectors from spec-tiler section 18.
    #[test]
    fn golden_canonical_json_and_digests() {
        let cases: Vec<(Canon, &str, &str)> = vec![
            (
                Canon::Object(vec![
                    (
                        "zoom_levels".into(),
                        Canon::Array(vec![Canon::Int(4), Canon::Int(5)]),
                    ),
                    ("tile_size".into(), Canon::Int(256)),
                ]),
                r#"{"tile_size":256,"zoom_levels":[4,5]}"#,
                "438fa04ff498b1d035ccb67effc4be5f998faf71f43ef32fc0582fe76cb18fcf",
            ),
            (
                Canon::Int(1),
                "1",
                "6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b",
            ),
            (
                Canon::Float(1.0),
                "1.0",
                "d0ff5974b6aa52cf562bea5921840c032a860a91a3512f7fe8f768f6bbe005f6",
            ),
            (
                Canon::Float(f64::NAN),
                r#"{"float":"nan"}"#,
                "c9772aeba1e20279e6937fa6a1ed65edca6e3e187fd1b2b2f7a3abb09cd02755",
            ),
            (
                Canon::Float(1e16),
                "1e+16",
                "a144838520595009e7daf5aff8472573f9ce6a4bcd8c30673675618883464ab0",
            ),
            (
                Canon::Float(1e-5),
                "1e-05",
                "5a9530d3c6d5faad6ba8afe16fb8e438df23865128370b2ec14ef622bb5f2ec9",
            ),
            (
                Canon::Float(0.01),
                "0.01",
                "312b95ee5a344d2f7a16ad817ff70788980da6e30b9bb04b651d53847abd476f",
            ),
            (
                Canon::Object(vec![("unit".into(), Canon::Str("\u{b5}g/m\u{b3}".into()))]),
                r#"{"unit":"\u00b5g/m\u00b3"}"#,
                "3a6376f348e82a364cb30a53071b3e4e1e49ae63ba72c3fa06fd8d501dd8c3ff",
            ),
            (
                Canon::Bytes(vec![0, 1]),
                r#"{"bytes":"0001"}"#,
                "6cd21a7f60c406a723e34bad3b8d23951608a5b73d6756b7334083518739ea25",
            ),
            (
                Canon::Object(vec![(
                    "category_map".into(),
                    Canon::Object(vec![
                        ("2".into(), Canon::Str("snow".into())),
                        ("1".into(), Canon::Str("rain".into())),
                    ]),
                )]),
                r#"{"category_map":{"1":"rain","2":"snow"}}"#,
                "bab941ea5e130a853e11776b5192fd9627d3e0855b1500b0ffdaa6dcc061c9ad",
            ),
        ];
        for (value, json, digest) in cases {
            assert_eq!(value.to_json(), json);
            assert_eq!(stable_digest(&value), digest, "{json}");
        }
    }

    #[test]
    fn golden_array_and_path_digests() {
        // np.array([[1,2],[3,4]], dtype=np.uint8)
        assert_eq!(
            array_digest_u8(&[1, 2, 3, 4], &[2, 2]),
            "f190ff2f8cf085c85ca01972ceab76a3a8392250045f1015471552a9ef85b6df"
        );
        // Float32 cast: np.float64(0.1).astype(np.float32) == 0.10000000149011612
        assert_eq!((0.1f64 as f32) as f64, 0.10000000149011612);
    }

    #[test]
    fn python_float_repr_matches_python_rules() {
        assert_eq!(python_float_repr(0.0), "0.0");
        assert_eq!(python_float_repr(-0.0), "-0.0");
        assert_eq!(python_float_repr(1.0), "1.0");
        assert_eq!(python_float_repr(123.456), "123.456");
        assert_eq!(python_float_repr(1e15), "1000000000000000.0");
        assert_eq!(python_float_repr(1.5e16), "1.5e+16");
        assert_eq!(python_float_repr(1.5e-7), "1.5e-07");
        assert_eq!(python_float_repr(0.0001), "0.0001");
        assert_eq!(python_float_repr(-2.5), "-2.5");
        assert_eq!(python_float_repr(1e100), "1e+100");
    }

    #[test]
    fn non_ascii_escapes_supplementary_characters_as_surrogates() {
        let mut out = String::new();
        write_json_string("\u{1F4A9}\u{7f}", &mut out);
        assert_eq!(out, r#""\ud83d\udca9\u007f""#);
    }

    #[test]
    fn identity_document_roundtrips_and_requires_ten_fields() {
        let id = sample_identity();
        let doc = serde_json::to_value(id.to_document()).unwrap();
        assert_eq!(PyramidIdentity::from_document(&doc).unwrap(), id);
        let mut extra = doc.clone();
        extra
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), Value::from("x"));
        assert!(PyramidIdentity::from_document(&extra).is_err());
        let mut empty = doc;
        empty
            .as_object_mut()
            .unwrap()
            .insert("renderer".into(), Value::from(""));
        assert!(PyramidIdentity::from_document(&empty).is_err());
    }

    #[test]
    fn cross_renderer_compatibility_is_strict() {
        let indexed = sample_identity();
        let mut legacy = indexed.clone();
        legacy.renderer = "legacy".into();
        legacy.algorithm = "rgba-bilinear-v1".into();
        assert!(legacy.is_renderer_compatible_with(&indexed));
        // Our own renderer name is never compatible with Python's `indexed`.
        let mut rs = indexed.clone();
        rs.renderer = RENDERER.into();
        assert!(!indexed.is_renderer_compatible_with(&rs));
        let mut changed = legacy.clone();
        changed.semantic_digest = "other".into();
        assert!(!changed.is_renderer_compatible_with(&indexed));
    }

    pub(crate) fn sample_identity() -> PyramidIdentity {
        PyramidIdentity {
            renderer: "indexed".into(),
            algorithm: "physical-bilinear-classify-v1".into(),
            source_id: "frame:1".into(),
            source_digest: "a".into(),
            grid_spec_digest: "b".into(),
            palette_name: "classic".into(),
            palette_digest: "c".into(),
            tile_spec_digest: "d".into(),
            semantic_digest: "e".into(),
            policy_digest: "f".into(),
        }
    }

    #[test]
    fn grid_identity_bits_treat_negative_zero_as_zero() {
        let fp = fingerprint_f64(&[1.0, 2.0]);
        let mk = |lat0: f64| GridIdentity {
            kind: "regular",
            height: 2,
            width: 2,
            lat_bounds: (lat0, 1.0),
            lon_bounds: (0.0, 1.0),
            coordinates: CoordinateIdentity::Regular {
                lats: fp.clone(),
                lons: fp.clone(),
            },
        };
        assert_eq!(mk(0.0), mk(-0.0));
        assert_ne!(grid_spec_digest(&mk(0.0)), grid_spec_digest(&mk(0.5)));
    }
}
