//! Packed data tiles v1 (`docs/packed-tiles.md`): quantized physical values, several frames per tile.
//!
//! A packed tile is an 8-bit RGB PNG atlas. Each channel of each 512-px block holds one frame (slot)
//! as `q = floor((value - offset) / scale)` in `1..=255`, with 0 for missing. Blocks of 3 slots are
//! laid out row-major, [`PACKED_COLS`] blocks wide. A bucket directory holds the tiles of up to
//! [`PACKED_MAX_SLOTS`] frames and a `.packed-bucket-v1.json` file `{"v": n, "slots": [...]}` that is
//! rewritten last.
//!
//! [`ValueSampler`] samples physical values with the same routing as the colored tiles (bilinear at
//! native zooms, max-pooled footprint at overview zooms). [`pack_frames`] merges new slots into a
//! bucket under a per-layer `flock`. Call it from a plain thread, never from a task of the Rayon pool
//! it uses (the lock waiter could be the worker the holder needs).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use radar_geo::Grid;
use rayon::prelude::*;
use rustix::fs::{FlockOperation, flock};
use serde_json::{Value, json};

use crate::error::{Result, TilerError};
use crate::geometry::{MAX_ZOOM, SourceGeometry};
use crate::render::{Overview, Sampler};
use crate::typed::{TYPE_CODE_STRIDE, TypePlane};

/// Edge of one block (one tile) of the atlas, in pixels.
pub const PACKED_TILE_SIZE: usize = 512;
/// Blocks per atlas row.
pub const PACKED_COLS: usize = 2;
/// Most slots in one bucket (8 blocks: a 1024 x 2048 atlas).
pub const PACKED_MAX_SLOTS: usize = 24;
/// Slots per block: one per RGB channel.
pub const SLOTS_PER_BLOCK: usize = 3;
/// Bucket file, written after every tile of an update is in place.
pub const BUCKET_FILE: &str = ".packed-bucket-v1.json";
/// Per-layer lock file that serializes bucket updates.
pub const LOCK_FILE: &str = ".packed.lock";

/// Linear quantization of one palette key (spec "Quantization per palette key").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantizer {
    /// Value step per code.
    pub scale: f64,
    /// Value at code 0 (code 0 itself means missing).
    pub offset: f64,
    /// Values below this are stored as 0: every palette leaves them transparent, and keeping the
    /// sub-threshold noise made real MRMS tiles several times larger. A floor below
    /// `offset + scale` stores the values between the two as code 1.
    pub floor: f64,
    /// Typed codes: `class * TYPE_CODE_STRIDE + min(q, TYPE_CODE_STRIDE - 1)` ([`Self::code`]).
    pub typed: bool,
}

impl Quantizer {
    /// The table entry for a palette `color_key`, or `None` for keys that are not packed in v1.
    pub fn for_color_key(color_key: &str) -> Option<Quantizer> {
        let (scale, offset, floor) = match color_key {
            "reflectivity" => (2.5, 0.0, 5.0),
            "temperature" => (1.0, -80.0, -79.0),
            "dewpoint" => (5.0, -80.0, -75.0),
            "wind_speed" => (5.0, -5.0, 0.0),
            "humidity" => (10.0, -10.0, 0.0),
            "cape" => (500.0, 0.0, 500.0),
            "precip_accum" => (0.05, 0.0, 0.01),
            "cloud_cover" => (5.0, 0.0, 10.0),
            "pm25" => (1.0, 0.5, 0.0),
            "ozone" => (1.0, 0.0, 0.0),
            _ => return None,
        };
        Some(Quantizer {
            scale,
            offset,
            floor,
            typed: false,
        })
    }

    /// The same quantizer with typed codes.
    pub fn with_types(self) -> Quantizer {
        Quantizer {
            typed: true,
            ..self
        }
    }

    /// The stored code of `value` with precipitation class `class`: `quantize(value)` for an untyped
    /// quantizer, else `class * 32 + min(q, 31)`, and 0 when `q` is 0.
    #[inline]
    pub fn code(&self, value: f32, class: u8) -> u8 {
        let q = self.quantize(value);
        if !self.typed || q == 0 {
            return q;
        }
        class.min(7) * TYPE_CODE_STRIDE + q.min(TYPE_CODE_STRIDE - 1)
    }

    /// NaN or a value below `floor` is 0; otherwise `q = clamp(floor((value - offset) / scale), 1,
    /// 255)`. Truncating keeps every value in its palette band when the band edges are multiples of
    /// `scale` from `offset`.
    #[inline]
    pub fn quantize(&self, value: f32) -> u8 {
        let v = f64::from(value);
        if v.is_nan() || v < self.floor {
            return 0;
        }
        let q = ((v - self.offset) / self.scale).floor();
        if q >= 255.0 {
            255
        } else if q >= 1.0 {
            q as u8
        } else {
            1
        }
    }

    /// The value a nonzero code stands for: `offset + q * scale`.
    pub fn value(&self, q: u8) -> f64 {
        self.offset + f64::from(q) * self.scale
    }
}

/// Where slot `s` lives in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotPosition {
    /// `s / 3`.
    pub block: usize,
    /// `s % 3`: R = 0, G = 1, B = 2.
    pub channel: usize,
    /// Left pixel of the block: `(block % cols) * tile`.
    pub x0: usize,
    /// Top pixel of the block: `(block / cols) * tile`.
    pub y0: usize,
}

/// Atlas address of slot `slot` for blocks of `tile` pixels.
pub fn slot_position(slot: usize, tile: usize) -> SlotPosition {
    let block = slot / SLOTS_PER_BLOCK;
    SlotPosition {
        block,
        channel: slot % SLOTS_PER_BLOCK,
        x0: (block % PACKED_COLS) * tile,
        y0: (block / PACKED_COLS) * tile,
    }
}

/// Atlas `(width, height)` in pixels for `n_slots` slots (at least one block row).
pub fn atlas_dims(n_slots: usize, tile: usize) -> (usize, usize) {
    let blocks = n_slots.div_ceil(SLOTS_PER_BLOCK).max(1);
    (tile * PACKED_COLS, tile * blocks.div_ceil(PACKED_COLS))
}

/// Physical-value sampler for one frame: the tiles of `zooms` that overlap the grid, each sampled
/// as the colored renderer samples it, but without classifying.
pub struct ValueSampler<'a> {
    inner: Sampler<'a>,
    types: Option<TypePlane>,
}

impl<'a> ValueSampler<'a> {
    /// Plan the tiles of `zooms` for `grid`. `nodata` marks missing cells besides NaN. `overview`
    /// max-pools zooms whose source cells are finer than a pixel (needs a regular grid).
    pub fn new(
        grid: &'a Grid,
        nodata: Option<f64>,
        zooms: &[u32],
        tile_size: usize,
        overview: Option<Overview>,
    ) -> Result<Self> {
        if tile_size == 0 || tile_size > 4096 {
            return Err(TilerError::Invalid("tile_size must be in 1..=4096".into()));
        }
        if zooms.iter().any(|&z| z > MAX_ZOOM) {
            return Err(TilerError::Invalid(format!(
                "zoom levels must be in 0..={MAX_ZOOM}"
            )));
        }
        if grid.height == 0 || grid.width == 0 || grid.values.len() != grid.height * grid.width {
            return Err(TilerError::Invalid(
                "grid values must be height x width with at least one cell".into(),
            ));
        }
        let src = SourceGeometry::from_grid(grid)?;
        let inner = Sampler::new(
            grid,
            src,
            nodata,
            1.0,
            false,
            overview.is_some(),
            tile_size,
            zooms,
        )?;
        Ok(Self { inner, types: None })
    }

    /// Attach a per-cell precipitation class plane (same shape as the grid).
    pub fn with_types(mut self, types: Option<TypePlane>) -> Result<Self> {
        if let Some(t) = &types
            && t.len() != self.inner.cell_count()
        {
            return Err(TilerError::Invalid(
                "type plane must have one class per grid cell".into(),
            ));
        }
        self.types = types;
        Ok(self)
    }

    /// Classes of tile `index` into `out` (`tile * tile`); all 0 without a type plane.
    pub fn sample_types(&self, index: usize, out: &mut Vec<u8>) -> Result<()> {
        match &self.types {
            Some(t) => self.inner.types_at(index, t.classes(), out),
            None => {
                out.clear();
                out.resize(self.tile_size() * self.tile_size(), 0);
                Ok(())
            }
        }
    }

    /// Tile edge in pixels.
    pub fn tile_size(&self) -> usize {
        self.inner.tile_size()
    }

    /// `(z, x, y)` of every tile that overlaps the grid bbox, in sampling order.
    pub fn tiles(&self) -> Vec<(u32, u32, u32)> {
        self.inner.tile_coords().collect()
    }

    /// Values of tile `index` of [`Self::tiles`] into `out` (`tile * tile`, row-major, NaN missing).
    /// `false` when the tile has no source coverage; `out` is then unspecified.
    pub fn sample(&self, index: usize, out: &mut Vec<f32>) -> Result<bool> {
        self.inner.values_at(index, out)
    }
}

/// One sampled tile: `(z, x, y)` and its `tile * tile` values.
pub type SampledTile = ((u32, u32, u32), Vec<f32>);

/// Sample every covered tile of `grid` at once. Holds all tiles in memory: for tests and tools.
pub fn sample_tiles(
    grid: &Grid,
    nodata: Option<f64>,
    zooms: &[u32],
    tile_size: usize,
    overview: Option<Overview>,
) -> Result<Vec<SampledTile>> {
    let sampler = ValueSampler::new(grid, nodata, zooms, tile_size, overview)?;
    let mut out = Vec::new();
    for (i, coord) in sampler.tiles().into_iter().enumerate() {
        let mut values = Vec::new();
        if sampler.sample(i, &mut values)? {
            out.push((coord, values));
        }
    }
    Ok(out)
}

/// Contents of a bucket's `.packed-bucket-v1.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketState {
    /// Update counter, starting at 1. Clients put it in the tile URL.
    pub v: u64,
    /// Slot timestamps, in slot order.
    pub slots: Vec<String>,
    /// The slots hold typed codes (`"types": 32` in the file).
    pub typed: bool,
}

impl BucketState {
    fn to_json(&self) -> Value {
        let mut doc = json!({"v": self.v, "slots": self.slots});
        if self.typed {
            doc["types"] = json!(TYPE_CODE_STRIDE);
        }
        doc
    }
}

/// Read `dir/.packed-bucket-v1.json`. `None` when the bucket has no file yet.
pub fn read_bucket(dir: &Path) -> Result<Option<BucketState>> {
    let text = match fs::read_to_string(dir.join(BUCKET_FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| TilerError::Corrupt(format!("{BUCKET_FILE}: {e}")))?;
    let v = value
        .get("v")
        .and_then(Value::as_u64)
        .ok_or_else(|| TilerError::Corrupt(format!("{BUCKET_FILE}: missing \"v\"")))?;
    let slots = value
        .get("slots")
        .and_then(Value::as_array)
        .ok_or_else(|| TilerError::Corrupt(format!("{BUCKET_FILE}: missing \"slots\"")))?
        .iter()
        .map(|s| {
            s.as_str()
                .map(str::to_string)
                .ok_or_else(|| TilerError::Corrupt(format!("{BUCKET_FILE}: non-string slot")))
        })
        .collect::<Result<Vec<_>>>()?;
    let typed = value.get("types").is_some_and(|t| !t.is_null());
    Ok(Some(BucketState { v, slots, typed }))
}

/// Every bucket under `layer_dir` that has a bucket file, as `(id, state)` sorted by id. A missing
/// `layer_dir` has none.
pub fn list_buckets(layer_dir: &Path) -> Result<Vec<(String, BucketState)>> {
    let entries = match fs::read_dir(layer_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || !path.is_dir() {
            continue;
        }
        if let Some(state) = read_bucket(&path)? {
            out.push((name.to_string(), state));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// One frame to pack: its slot timestamp and its sampled values.
pub struct PackFrame<'s, 'g> {
    /// Slot timestamp, the same string as the manifest frame's `timestamp`.
    pub timestamp: &'s str,
    /// The frame's values.
    pub sampler: &'s ValueSampler<'g>,
}

/// Result of [`pack_frames`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackOutcome {
    /// The bucket file now says this. `tiles_written` counts rewritten tile files.
    Written {
        /// The new bucket state.
        bucket: BucketState,
        /// Tile files written (or removed because they became empty).
        tiles_written: u64,
    },
    /// The new slots would exceed [`PACKED_MAX_SLOTS`]. Nothing was written.
    Full,
}

/// Merge `frames` into bucket `bucket_id` under `layer_dir` (`TILE_DIR/packed/<layer>`).
///
/// A timestamp already in the bucket keeps its slot and is rewritten; a new one takes the next slot.
/// Every tile that changes is decoded, updated in its slot channels and re-encoded, with the atlas
/// grown to the new slot count. Tiles whose every slot is 0 are not written. The bucket file is
/// written last with `v` incremented. Holds `layer_dir/.packed.lock` throughout and runs the tiles
/// on the current Rayon pool.
pub fn pack_frames(
    layer_dir: &Path,
    bucket_id: &str,
    frames: &[PackFrame<'_, '_>],
    quant: Quantizer,
) -> Result<PackOutcome> {
    validate_bucket_id(bucket_id)?;
    let tile = match frames.first() {
        Some(f) => f.sampler.tile_size(),
        None => return Err(TilerError::Invalid("no frames to pack".into())),
    };
    if frames.iter().any(|f| f.sampler.tile_size() != tile) {
        return Err(TilerError::Invalid(
            "every packed frame must use the same tile size".into(),
        ));
    }
    fs::create_dir_all(layer_dir)?;
    let _lock = LayerLock::acquire(&layer_dir.join(LOCK_FILE))?;
    let bucket_dir = layer_dir.join(bucket_id);
    let previous = read_bucket(&bucket_dir)?;
    if previous.as_ref().is_some_and(|b| b.typed != quant.typed) {
        // The slots of one bucket share one code space; the next bucket starts with the new one.
        return Err(TilerError::Invalid(format!(
            "bucket {bucket_id} holds {} codes",
            if quant.typed { "untyped" } else { "typed" }
        )));
    }
    let mut slots = previous
        .as_ref()
        .map(|b| b.slots.clone())
        .unwrap_or_default();
    let mut assigned = Vec::with_capacity(frames.len());
    let mut rewrite = false;
    for frame in frames {
        match slots.iter().position(|s| s == frame.timestamp) {
            Some(slot) => {
                rewrite = true;
                assigned.push(slot);
            }
            None => {
                slots.push(frame.timestamp.to_string());
                assigned.push(slots.len() - 1);
            }
        }
    }
    if slots.len() > PACKED_MAX_SLOTS {
        return Ok(PackOutcome::Full);
    }
    let dims = atlas_dims(slots.len(), tile);

    // Every tile a frame covers, plus every tile already in the bucket (it may need to grow).
    let mut work: BTreeMap<(u32, u32, u32), Vec<Option<usize>>> = BTreeMap::new();
    for (f, frame) in frames.iter().enumerate() {
        for (i, coord) in frame.sampler.tiles().into_iter().enumerate() {
            work.entry(coord)
                .or_insert_with(|| vec![None; frames.len()])[f] = Some(i);
        }
    }
    for coord in existing_tiles(&bucket_dir)? {
        work.entry(coord)
            .or_insert_with(|| vec![None; frames.len()]);
    }
    let work: Vec<_> = work.into_iter().collect();
    let ctx = MergeCtx {
        bucket_dir: &bucket_dir,
        frames,
        assigned: &assigned,
        quant,
        tile,
        dims,
        rewrite,
    };
    let tiles_written = work
        .par_iter()
        .map_init(
            || (Vec::new(), Vec::new(), Vec::new()),
            |(values, types, planes), (coord, indices)| {
                ctx.merge_tile(*coord, indices, values, types, planes)
            },
        )
        .try_reduce(|| 0u64, |a, b| Ok(a + b))?;

    let bucket = BucketState {
        v: previous.map_or(0, |b| b.v) + 1,
        slots,
        typed: quant.typed,
    };
    fs::create_dir_all(&bucket_dir)?;
    let text = serde_json::to_string(&bucket.to_json())
        .map_err(|e| TilerError::Invalid(format!("bucket file: {e}")))?;
    write_atomic(&bucket_dir.join(BUCKET_FILE), text.as_bytes())?;
    Ok(PackOutcome::Written {
        bucket,
        tiles_written,
    })
}

/// A bucket id is one path segment that cannot collide with the lock or bucket files.
fn validate_bucket_id(id: &str) -> Result<()> {
    let bad =
        id.is_empty() || id.starts_with('.') || id.contains(['/', '\\', '\0']) || id.len() > 200;
    if bad {
        return Err(TilerError::Invalid(format!("invalid bucket id {id:?}")));
    }
    Ok(())
}

struct MergeCtx<'a, 's, 'g> {
    bucket_dir: &'a Path,
    frames: &'a [PackFrame<'s, 'g>],
    assigned: &'a [usize],
    quant: Quantizer,
    tile: usize,
    dims: (usize, usize),
    rewrite: bool,
}

impl MergeCtx<'_, '_, '_> {
    /// Update one tile file. Returns 1 when the file was written or removed.
    fn merge_tile(
        &self,
        (z, x, y): (u32, u32, u32),
        indices: &[Option<usize>],
        values: &mut Vec<f32>,
        types: &mut Vec<u8>,
        planes: &mut Vec<u8>,
    ) -> Result<u64> {
        let t = self.tile;
        let plane_len = t * t;
        planes.clear();
        planes.resize(plane_len * self.frames.len(), 0);
        let mut any_data = false;
        for (f, index) in indices.iter().enumerate() {
            let Some(index) = *index else { continue };
            if !self.frames[f].sampler.sample(index, values)? {
                continue;
            }
            let plane = &mut planes[f * plane_len..(f + 1) * plane_len];
            if self.quant.typed {
                self.frames[f].sampler.sample_types(index, types)?;
                for ((q, &v), &c) in plane.iter_mut().zip(values.iter()).zip(types.iter()) {
                    *q = self.quant.code(v, c);
                }
            } else {
                for (q, &v) in plane.iter_mut().zip(values.iter()) {
                    *q = self.quant.quantize(v);
                }
            }
            any_data |= plane.iter().any(|&q| q != 0);
        }
        let path = tile_path(self.bucket_dir, z, x, y);
        let existing = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let (w, h) = self.dims;
        let mut atlas = match &existing {
            None if !any_data => return Ok(0),
            None => vec![0u8; w * h * 3],
            Some(bytes) => {
                let (ew, eh, data) = decode_rgb8(bytes)?;
                if !any_data && !self.rewrite && (ew, eh) == (w, h) {
                    return Ok(0);
                }
                if ew != w || eh > h {
                    return Err(TilerError::Corrupt(format!(
                        "packed tile {} is {ew}x{eh}, expected {w}x<={h}",
                        path.display()
                    )));
                }
                let mut atlas = data;
                atlas.resize(w * h * 3, 0);
                atlas
            }
        };
        for (f, &slot) in self.assigned.iter().enumerate() {
            let plane = &planes[f * plane_len..(f + 1) * plane_len];
            write_plane(&mut atlas, w, slot_position(slot, t), t, plane);
        }
        if atlas.iter().all(|&b| b == 0) {
            // Only reachable on a rewrite that emptied the tile.
            if existing.is_some() {
                fs::remove_file(&path)?;
                return Ok(1);
            }
            return Ok(0);
        }
        let png = encode_rgb8(&atlas, w, h)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&path, &png)?;
        Ok(1)
    }
}

/// Copy a `tile x tile` plane into one channel of an RGB atlas `width` pixels wide.
fn write_plane(atlas: &mut [u8], width: usize, pos: SlotPosition, tile: usize, plane: &[u8]) {
    for (py, row) in plane.chunks_exact(tile).enumerate() {
        let start = ((pos.y0 + py) * width + pos.x0) * 3;
        let dst = &mut atlas[start..start + tile * 3];
        for (texel, &q) in dst.chunks_exact_mut(3).zip(row) {
            texel[pos.channel] = q;
        }
    }
}

/// `bucket_dir/<z>/<x>/<y>.png`.
pub fn tile_path(bucket_dir: &Path, z: u32, x: u32, y: u32) -> PathBuf {
    bucket_dir
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{y}.png"))
}

/// `(z, x, y)` of every tile file already in the bucket.
fn existing_tiles(bucket_dir: &Path) -> Result<Vec<(u32, u32, u32)>> {
    let mut out = Vec::new();
    for (z, zdir) in numeric_children(bucket_dir, "")? {
        for (x, xdir) in numeric_children(&zdir, "")? {
            for (y, _) in numeric_children(&xdir, ".png")? {
                out.push((z, x, y));
            }
        }
    }
    Ok(out)
}

/// Children of `dir` named `<u32><suffix>`. A missing `dir` has none.
fn numeric_children(dir: &Path, suffix: &str) -> Result<Vec<(u32, PathBuf)>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(suffix) else {
            continue;
        };
        if !stem.is_empty()
            && stem.bytes().all(|b| b.is_ascii_digit())
            && let Ok(n) = stem.parse()
        {
            out.push((n, path));
        }
    }
    Ok(out)
}

/// Write `bytes` to a temporary name next to `path`, then rename it into place.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| TilerError::Invalid(format!("bad path {}", path.display())))?;
    let tmp = path.with_file_name(format!(".{name}.tmp-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| -> std::io::Result<()> {
        let mut file = BufWriter::new(File::create(&tmp)?);
        file.write_all(bytes)?;
        file.into_inner().map_err(|e| e.into_error())?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}

/// Encode an 8-bit RGB PNG: color type 2, no alpha, no gamma or color-profile chunks.
pub fn encode_rgb8(data: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Balanced);
    let mut writer = encoder
        .write_header()
        .map_err(|e| TilerError::Invalid(format!("png encode: {e}")))?;
    writer
        .write_image_data(data)
        .map_err(|e| TilerError::Invalid(format!("png encode: {e}")))?;
    writer
        .finish()
        .map_err(|e| TilerError::Invalid(format!("png encode: {e}")))?;
    Ok(out)
}

/// Decode an 8-bit RGB PNG into `(width, height, rgb bytes)`.
pub fn decode_rgb8(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>)> {
    let corrupt = |e: png::DecodingError| TilerError::Corrupt(format!("packed png decode: {e}"));
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(corrupt)?;
    let info = reader.info();
    if info.color_type != png::ColorType::Rgb || info.bit_depth != png::BitDepth::Eight {
        return Err(TilerError::Corrupt("packed tile is not 8-bit RGB".into()));
    }
    let len = reader
        .output_buffer_size()
        .ok_or_else(|| TilerError::Corrupt("png output buffer too large".into()))?;
    let mut data = vec![0u8; len];
    let frame = reader.next_frame(&mut data).map_err(corrupt)?;
    data.truncate(frame.buffer_size());
    Ok((frame.width as usize, frame.height as usize, data))
}

/// Exclusive `flock` on the layer's lock file, released on drop.
struct LayerLock {
    file: File,
}

impl LayerLock {
    fn acquire(path: &Path) -> Result<Self> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        loop {
            match flock(&file, FlockOperation::LockExclusive) {
                Ok(()) => return Ok(Self { file }),
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(std::io::Error::from(e).into()),
            }
        }
    }
}

impl Drop for LayerLock {
    fn drop(&mut self) {
        let _ = flock(&self.file, FlockOperation::Unlock);
    }
}

#[cfg(test)]
mod tests;
