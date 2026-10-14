//! `radar parity`: compare two tile pyramids (Python build against Rust build) tile by tile.
//!
//! Both stacks write the same `TILE_DIR` layout (spec-tiler §12):
//! `{root}/{layer}/{palette}/{frame path}/{z}/{x}/{y}.png`, where the frame path is the manifest
//! frame's `path` (the timestamp for MRMS, `runs/<run>/<timestamp>` for HRRR). Hidden entries
//! (`.render-publish-locks`, `.<ts>.tmp-<hex>` staging dirs, marker files) are skipped.
//!
//! For every `(layer, palette, frame)` present on both sides the comparison reports the tile-set
//! difference, decodes every PNG to RGBA (indexed and RGBA encodings alike, on rayon), and counts
//! identical tiles, identical pixels, the largest channel delta, and what kind of difference each
//! differing pixel is. A tile that exists on one side only is compared against a fully transparent
//! tile, because that is what a client sees for a 404: the renderers only elide fully transparent
//! tiles. When both tiles are indexed, the palette-index rasters are compared too.
//!
//! The pass/fail threshold is the share of visibly differing pixels per frame. The default
//! ([`DEFAULT_MAX_DIFF_PCT`]) accepts the known difference between Python's production *legacy RGBA*
//! renderer and the Rust *indexed* renderer (about 92% of pixels identical, by design: legacy
//! samples colour before classifying). `--exact` is for comparing against Python's *indexed*
//! renderer, which matches Rust exactly.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use rayon::prelude::*;
use serde::Serialize;
use serde_json::{Map, Value};

/// Default `--max-diff-pct`: the share of visibly differing pixels a frame may have and still pass.
///
/// Calibrated on the bench (`bench/results/latest.md`): Python legacy RGBA against Rust indexed is
/// 92.1% pixel-identical over 585 tiles, so 7.9% of pixels differ. 10% leaves a small margin.
pub const DEFAULT_MAX_DIFF_PCT: f64 = 10.0;

/// How many tile or frame names each list in the report carries before it is cut.
pub const SAMPLE_LIMIT: usize = 20;

const PARITY_LONG_HELP: &str = "\
Layout: <root>/<layer>/<palette>/<frame path>/<z>/<x>/<y>.png on both sides (the TILE_DIR
layout). Every (layer, palette, frame) present on both sides is compared. A tile present on one
side only is compared against a fully transparent tile (what a client sees for a 404).

Threshold and the two Python renderers:
  Python production writes LEGACY RGBA tiles (TILE_RENDERER=legacy). It samples colour before
  classifying, so against Rust's indexed tiles only about 92% of pixels are identical (bench:
  92.1%), and the tile sets differ slightly (legacy keeps some edge tiles indexed elides). The
  default --max-diff-pct 10 accepts that difference and still catches a broken frame.
  Python's INDEXED renderer (renderer=\"indexed\") is the algorithm Rust implements, and the
  output is identical: same tile set, same palette indices, same RGBA. Use --exact for that
  comparison.

Pixel kinds (for differing pixels):
  transparent_rgb_only  both transparent, RGB differs (invisible, never counted as a difference)
  alpha_edge            transparent on one side only (coverage or transparency edge)
  alpha_only            both visible, same RGB, different alpha
  color                 both visible, different RGB

Exit status: 0 pass (differences within the threshold are reported as warnings), 1 a frame
failed (over the threshold, undecodable or size-mismatched tile, or an --exact violation), 2
nothing to compare (no frame present on both sides) or an error (unreadable root).

Manifests: --python-manifest / --rust-manifest take manifest.json or the state dir holding it;
without them <root>/manifest.json is used when it exists. Frame lists, paths, palettes and keys are
compared. Differences are reported; with --exact, entry mismatches (not frame-list timing) fail.";

/// Arguments of `radar parity`.
#[derive(Debug, Clone, Args)]
#[command(after_long_help = PARITY_LONG_HELP)]
pub struct ParityArgs {
    /// Tile root written by the Python build (TILE_DIR layout).
    #[arg(long, value_name = "DIR")]
    pub python_root: PathBuf,
    /// Tile root written by the Rust build (TILE_DIR layout).
    #[arg(long, value_name = "DIR")]
    pub rust_root: PathBuf,
    /// Only this layer (product), e.g. `radar`, `nowcast`. Repeatable.
    #[arg(long = "layer", visible_alias = "product", value_name = "LAYER")]
    pub layers: Vec<String>,
    /// Only this palette, e.g. `classic`. Repeatable.
    #[arg(long = "palette", value_name = "PALETTE")]
    pub palettes: Vec<String>,
    /// Only this frame: the frame path, its last segment, or its timestamp (`Z` or `+00:00`). Repeatable.
    #[arg(long = "frame", value_name = "FRAME")]
    pub frames: Vec<String>,
    /// Lowest zoom to compare.
    #[arg(long, value_name = "Z")]
    pub min_zoom: Option<u32>,
    /// Highest zoom to compare.
    #[arg(long, value_name = "Z")]
    pub max_zoom: Option<u32>,
    /// Fail a frame when more than this percentage of its pixels differ visibly. The default (10)
    /// accepts Python legacy RGBA against Rust indexed (~92% identical by design). Use --exact (or
    /// 0) against Python's indexed renderer, which must match exactly.
    #[arg(long, value_name = "PCT", default_value_t = DEFAULT_MAX_DIFF_PCT)]
    pub max_diff_pct: f64,
    /// Exact mode, for Python's indexed renderer: no differing pixel, identical tile sets, identical
    /// palette indices where both tiles are indexed, and matching manifest entries.
    #[arg(long, conflicts_with = "max_diff_pct")]
    pub exact: bool,
    /// Python manifest.json, or the state directory holding it.
    #[arg(long, value_name = "PATH")]
    pub python_manifest: Option<PathBuf>,
    /// Rust manifest.json, or the state directory holding it.
    #[arg(long, value_name = "PATH")]
    pub rust_manifest: Option<PathBuf>,
    /// Machine-readable output (one JSON document).
    #[arg(long)]
    pub json: bool,
    /// Decoder threads (default: all cores).
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
}

// ------------------------------------------------------------------------------------------------
// Selection and discovery
// ------------------------------------------------------------------------------------------------

/// Which part of the pyramids to compare.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Layers to keep (empty: all).
    pub layers: Vec<String>,
    /// Palettes to keep (empty: all).
    pub palettes: Vec<String>,
    /// Frames to keep (empty: all). See [`Filter::frame_matches`].
    pub frames: Vec<String>,
    /// Lowest zoom kept.
    pub min_zoom: Option<u32>,
    /// Highest zoom kept.
    pub max_zoom: Option<u32>,
}

impl Filter {
    fn layer_ok(&self, layer: &str) -> bool {
        self.layers.is_empty() || self.layers.iter().any(|l| l == layer)
    }

    fn palette_ok(&self, palette: &str) -> bool {
        self.palettes.is_empty() || self.palettes.iter().any(|p| p == palette)
    }

    fn zoom_ok(&self, z: u32) -> bool {
        self.min_zoom.is_none_or(|m| z >= m) && self.max_zoom.is_none_or(|m| z <= m)
    }

    /// A frame path matches a `--frame` value when it equals it, ends with `/<value>`, or does so
    /// after the value is canonicalised as a UTC timestamp (`...Z` becomes `...+00:00`).
    pub fn frame_matches(&self, path: &str) -> bool {
        if self.frames.is_empty() {
            return true;
        }
        self.frames.iter().any(|want| {
            let canon = radar_core::manifest::canonical_timestamp(want).ok();
            [Some(want.as_str()), canon.as_deref()]
                .into_iter()
                .flatten()
                .any(|w| path == w || path.ends_with(&format!("/{w}")))
        })
    }
}

/// One pyramid: a frame of one layer in one palette.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct FrameKey {
    /// Layer (product) name.
    pub layer: String,
    /// Palette name.
    pub palette: String,
    /// Frame path below the palette directory (`/`-separated).
    pub path: String,
}

impl std::fmt::Display for FrameKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.layer, self.palette, self.path)
    }
}

/// `(z, x, y)` of a tile.
pub type TileKey = (u32, u32, u32);

/// Every tile of a tile root, by pyramid.
#[derive(Debug, Default)]
pub struct TreeScan {
    /// Tiles per pyramid.
    pub frames: BTreeMap<FrameKey, BTreeMap<TileKey, PathBuf>>,
    /// `.png` files that are not at a `<frame>/<z>/<x>/<y>.png` position.
    pub ignored_files: usize,
}

fn is_hidden_or_staging(name: &str) -> bool {
    name.starts_with('.') || name.contains(".tmp")
}

fn sorted_dirs(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // `Path::is_dir` follows symlinks, so a comparison root can be assembled from links.
        if is_hidden_or_staging(&name) || !entry.path().is_dir() {
            continue;
        }
        out.push((name, entry.path()));
    }
    out.sort();
    Ok(out)
}

/// Walk `{root}/{layer}/{palette}/**/{z}/{x}/{y}.png` and group the tiles by pyramid.
pub fn scan_tree(root: &Path, filter: &Filter) -> Result<TreeScan> {
    if !root.is_dir() {
        bail!("tile root {} is not a directory", root.display());
    }
    let mut scan = TreeScan::default();
    for (layer, layer_dir) in sorted_dirs(root)? {
        if !filter.layer_ok(&layer) {
            continue;
        }
        for (palette, palette_dir) in sorted_dirs(&layer_dir)? {
            if !filter.palette_ok(&palette) {
                continue;
            }
            let walker = walkdir::WalkDir::new(&palette_dir)
                .min_depth(1)
                .follow_links(true)
                .into_iter()
                .filter_entry(|e| !e.file_name().to_str().is_none_or(is_hidden_or_staging));
            for entry in walker {
                let entry = entry.with_context(|| format!("walk {}", palette_dir.display()))?;
                if !entry.file_type().is_file() {
                    continue;
                }
                let Some(rel) = entry
                    .path()
                    .strip_prefix(&palette_dir)
                    .ok()
                    .and_then(Path::to_str)
                else {
                    continue;
                };
                if !rel.ends_with(".png") {
                    continue;
                }
                let parts: Vec<&str> = rel.split(std::path::MAIN_SEPARATOR).collect();
                let Some((frame, z, x, y)) = split_tile_path(&parts) else {
                    scan.ignored_files += 1;
                    continue;
                };
                if !filter.zoom_ok(z) || !filter.frame_matches(&frame) {
                    continue;
                }
                scan.frames
                    .entry(FrameKey {
                        layer: layer.clone(),
                        palette: palette.clone(),
                        path: frame,
                    })
                    .or_default()
                    .insert((z, x, y), entry.into_path());
            }
        }
    }
    Ok(scan)
}

fn split_tile_path(parts: &[&str]) -> Option<(String, u32, u32, u32)> {
    if parts.len() < 4 {
        return None;
    }
    let n = parts.len();
    let y = parts[n - 1].strip_suffix(".png")?.parse().ok()?;
    let x = parts[n - 2].parse().ok()?;
    let z = parts[n - 3].parse().ok()?;
    Some((parts[..n - 3].join("/"), z, x, y))
}

fn tile_label((z, x, y): TileKey) -> String {
    format!("{z}/{x}/{y}.png")
}

// ------------------------------------------------------------------------------------------------
// Decoding
// ------------------------------------------------------------------------------------------------

/// A tile decoded to 8-bit RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedTile {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA, 4 bytes per pixel.
    pub rgba: Vec<u8>,
    /// Palette indices (one per pixel) when the PNG is indexed.
    pub indices: Option<Vec<u8>>,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const COLOR_TYPE_INDEXED: u8 = 3;

/// Decode any PNG to RGBA: indexed (PLTE + tRNS), grey, grey+alpha, RGB, RGBA, 8 or 16 bit.
pub fn decode_rgba(bytes: &[u8]) -> Result<DecodedTile> {
    if bytes.len() < 33 || bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        bail!("not a PNG file");
    }
    // IHDR data starts at byte 16: width, height, bit depth, colour type.
    if bytes[25] == COLOR_TYPE_INDEXED {
        decode_indexed(bytes)
    } else {
        decode_direct(bytes)
    }
}

fn decode_indexed(bytes: &[u8]) -> Result<DecodedTile> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().context("png header")?;
    let info = reader.info();
    let plte = info.palette.as_deref().unwrap_or(&[]).to_vec();
    let trns = info.trns.as_deref().unwrap_or(&[]).to_vec();
    let depth = info.bit_depth as usize;
    if !matches!(depth, 1 | 2 | 4 | 8) {
        bail!("indexed PNG with bit depth {depth}");
    }
    let size = reader
        .output_buffer_size()
        .context("png output buffer too large")?;
    let mut raw = vec![0u8; size];
    let frame = reader.next_frame(&mut raw).context("png data")?;
    let (w, h, line) = (frame.width as usize, frame.height as usize, frame.line_size);
    // Index -> RGBA. Entries past the PLTE are opaque black; entries past tRNS are opaque.
    let lut: Vec<[u8; 4]> = (0..256usize)
        .map(|i| {
            let rgb = plte.get(3 * i..3 * i + 3).unwrap_or(&[0, 0, 0]);
            [rgb[0], rgb[1], rgb[2], trns.get(i).copied().unwrap_or(255)]
        })
        .collect();
    let mut indices = Vec::with_capacity(w * h);
    if depth == 8 {
        for y in 0..h {
            indices.extend_from_slice(&raw[y * line..y * line + w]);
        }
    } else {
        let per_byte = 8 / depth;
        let mask = ((1u16 << depth) - 1) as u8;
        for y in 0..h {
            let row = &raw[y * line..(y + 1) * line];
            indices.extend((0..w).map(|x| {
                let shift = 8 - depth * (x % per_byte + 1);
                (row[x / per_byte] >> shift) & mask
            }));
        }
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for &i in &indices {
        rgba.extend_from_slice(&lut[i as usize]);
    }
    Ok(DecodedTile {
        width: frame.width,
        height: frame.height,
        rgba,
        indices: Some(indices),
    })
}

fn decode_direct(bytes: &[u8]) -> Result<DecodedTile> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().context("png header")?;
    let size = reader
        .output_buffer_size()
        .context("png output buffer too large")?;
    let mut buf = vec![0u8; size];
    let frame = reader.next_frame(&mut buf).context("png data")?;
    if frame.bit_depth != png::BitDepth::Eight {
        bail!("unexpected decoded bit depth {:?}", frame.bit_depth);
    }
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => bail!("palette was not expanded"),
    };
    let (w, h, line) = (frame.width as usize, frame.height as usize, frame.line_size);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &buf[y * line..y * line + w * channels];
        for p in row.chunks_exact(channels) {
            match channels {
                1 => rgba.extend_from_slice(&[p[0], p[0], p[0], 255]),
                2 => rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]),
                3 => rgba.extend_from_slice(&[p[0], p[1], p[2], 255]),
                _ => rgba.extend_from_slice(p),
            }
        }
    }
    Ok(DecodedTile {
        width: frame.width,
        height: frame.height,
        rgba,
        indices: None,
    })
}

// ------------------------------------------------------------------------------------------------
// Pixel comparison
// ------------------------------------------------------------------------------------------------

/// What kind of difference each differing pixel is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DiffKinds {
    /// Both transparent, RGB differs. Invisible; never counted as a difference.
    pub transparent_rgb_only: u64,
    /// Transparent on one side only: a coverage or transparency edge.
    pub alpha_edge: u64,
    /// Both visible, same RGB, different alpha.
    pub alpha_only: u64,
    /// Both visible, different RGB.
    pub color: u64,
}

impl DiffKinds {
    /// Pixels that look different (everything but `transparent_rgb_only`).
    pub fn visible(&self) -> u64 {
        self.alpha_edge + self.alpha_only + self.color
    }

    fn add(&mut self, o: &DiffKinds) {
        self.transparent_rgb_only += o.transparent_rgb_only;
        self.alpha_edge += o.alpha_edge;
        self.alpha_only += o.alpha_only;
        self.color += o.color;
    }
}

/// Pixel counts for one tile pair or an aggregate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PixelStats {
    /// Pixels compared.
    pub pixels: u64,
    /// Pixels whose RGBA bytes are equal.
    pub identical: u64,
    /// Differing pixels by kind.
    pub kinds: DiffKinds,
    /// Largest absolute channel difference over visibly differing pixels.
    pub max_channel_delta: u8,
}

impl PixelStats {
    fn add(&mut self, o: &PixelStats) {
        self.pixels += o.pixels;
        self.identical += o.identical;
        self.kinds.add(&o.kinds);
        self.max_channel_delta = self.max_channel_delta.max(o.max_channel_delta);
    }
}

/// Compare two RGBA buffers of the same size pixel by pixel.
pub fn compare_rgba(a: &[u8], b: &[u8]) -> PixelStats {
    debug_assert_eq!(a.len(), b.len());
    let mut s = PixelStats::default();
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        s.pixels += 1;
        if pa == pb {
            s.identical += 1;
            continue;
        }
        let (aa, ab) = (pa[3], pb[3]);
        if aa == 0 && ab == 0 {
            s.kinds.transparent_rgb_only += 1;
            continue;
        }
        if aa == 0 || ab == 0 {
            s.kinds.alpha_edge += 1;
        } else if pa[..3] == pb[..3] {
            s.kinds.alpha_only += 1;
        } else {
            s.kinds.color += 1;
        }
        let delta = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        s.max_channel_delta = s.max_channel_delta.max(delta);
    }
    s
}

#[derive(Debug)]
enum Side {
    Both {
        rgba_identical: bool,
        index_equal: Option<bool>,
    },
    OnlyPython,
    OnlyRust,
    SizeMismatch(String),
    DecodeError(String),
}

#[derive(Debug)]
struct TileOutcome {
    frame: usize,
    key: TileKey,
    side: Side,
    stats: PixelStats,
}

struct Job<'a> {
    frame: usize,
    key: TileKey,
    python: Option<&'a Path>,
    rust: Option<&'a Path>,
}

fn load(path: &Path) -> Result<DecodedTile> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    decode_rgba(&bytes).with_context(|| format!("decode {}", path.display()))
}

fn run_job(job: &Job<'_>) -> TileOutcome {
    let outcome = |side, stats| TileOutcome {
        frame: job.frame,
        key: job.key,
        side,
        stats,
    };
    let decode = |p: &Path| load(p).map_err(|e| format!("{e:#}"));
    match (job.python, job.rust) {
        (Some(p), Some(r)) => match (decode(p), decode(r)) {
            (Ok(a), Ok(b)) => {
                if (a.width, a.height) != (b.width, b.height) {
                    let msg = format!(
                        "{}: python {}x{}, rust {}x{}",
                        tile_label(job.key),
                        a.width,
                        a.height,
                        b.width,
                        b.height
                    );
                    return outcome(Side::SizeMismatch(msg), PixelStats::default());
                }
                let stats = compare_rgba(&a.rgba, &b.rgba);
                let index_equal = match (&a.indices, &b.indices) {
                    (Some(x), Some(y)) => Some(x == y),
                    _ => None,
                };
                outcome(
                    Side::Both {
                        rgba_identical: stats.identical == stats.pixels,
                        index_equal,
                    },
                    stats,
                )
            }
            (Err(e), _) | (_, Err(e)) => outcome(Side::DecodeError(e), PixelStats::default()),
        },
        (Some(path), None) | (None, Some(path)) => {
            let only_python = job.python.is_some();
            match decode(path) {
                Ok(t) => {
                    let empty = vec![0u8; t.rgba.len()];
                    let stats = if only_python {
                        compare_rgba(&t.rgba, &empty)
                    } else {
                        compare_rgba(&empty, &t.rgba)
                    };
                    let side = if only_python {
                        Side::OnlyPython
                    } else {
                        Side::OnlyRust
                    };
                    outcome(side, stats)
                }
                Err(e) => outcome(Side::DecodeError(e), PixelStats::default()),
            }
        }
        (None, None) => outcome(
            Side::DecodeError("no tile on either side".into()),
            PixelStats::default(),
        ),
    }
}

// ------------------------------------------------------------------------------------------------
// Report
// ------------------------------------------------------------------------------------------------

/// Check status, as in the debug harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// No difference.
    Ok,
    /// Differences within the threshold.
    Warn,
    /// Over the threshold, or a hard error.
    Fail,
}

impl Status {
    fn glyph(self) -> &'static str {
        match self {
            Status::Ok => "✓",
            Status::Warn => "⚠",
            Status::Fail => "✗",
        }
    }
}

/// Comparison of one pyramid present on both sides.
#[derive(Debug, Clone, Serialize)]
pub struct FrameReport {
    /// Layer name.
    pub layer: String,
    /// Palette name.
    pub palette: String,
    /// Frame path.
    pub path: String,
    /// Result for this frame.
    pub status: Status,
    /// Why the status is not `ok`.
    pub problems: Vec<String>,
    /// Tiles on the Python side.
    pub python_tiles: usize,
    /// Tiles on the Rust side.
    pub rust_tiles: usize,
    /// Tiles on both sides.
    pub common_tiles: usize,
    /// Tiles on the Python side only.
    pub only_python_count: usize,
    /// Tiles on the Rust side only.
    pub only_rust_count: usize,
    /// First [`SAMPLE_LIMIT`] Python-only tiles (`z/x/y.png`).
    pub only_python: Vec<String>,
    /// First [`SAMPLE_LIMIT`] Rust-only tiles.
    pub only_rust: Vec<String>,
    /// Common tiles whose decoded RGBA is byte-identical.
    pub identical_tiles: usize,
    /// Common tiles with no visible difference (identical, or differing only under alpha 0).
    pub equivalent_tiles: usize,
    /// Common tiles indexed on both sides.
    pub index_compared_tiles: usize,
    /// Of those, tiles with identical palette indices.
    pub index_identical_tiles: usize,
    /// First [`SAMPLE_LIMIT`] common tiles that differ visibly.
    pub differing_tiles: Vec<String>,
    /// Tiles whose dimensions differ.
    pub size_mismatch: Vec<String>,
    /// Tiles that could not be read or decoded.
    pub decode_errors: Vec<String>,
    /// Pixels compared (common tiles plus one-sided tiles against transparent).
    pub pixels_compared: u64,
    /// Pixels with byte-identical RGBA.
    pub pixels_identical: u64,
    /// Visibly differing pixels.
    pub pixels_differing: u64,
    /// `100 - diff_pct`: pixels that look the same.
    pub identical_pct: f64,
    /// Visibly differing pixels as a percentage of `pixels_compared`.
    pub diff_pct: f64,
    /// Largest absolute channel difference over visibly differing pixels.
    pub max_channel_delta: u8,
    /// Differing pixels by kind.
    pub diff_kinds: DiffKinds,
    /// True when every visible difference is an alpha or transparency edge (no colour change).
    pub transparency_only: bool,
}

/// Manifest difference for one frame present in both manifests.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestFrameDiff {
    /// Frame timestamp.
    pub timestamp: String,
    /// `[python, rust]` when the paths differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<[String; 2]>,
    /// `[python, rust]` when the palettes differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palettes: Option<[Vec<String>; 2]>,
    /// Frame keys only Python writes.
    pub keys_only_python: Vec<String>,
    /// Frame keys only Rust writes.
    pub keys_only_rust: Vec<String>,
    /// Keys on both sides with different values (informational).
    pub values_differ: Vec<String>,
}

impl ManifestFrameDiff {
    fn is_mismatch(&self) -> bool {
        self.path.is_some()
            || self.palettes.is_some()
            || !self.keys_only_python.is_empty()
            || !self.keys_only_rust.is_empty()
    }
}

/// Manifest difference for one layer.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestLayerDiff {
    /// Layer name.
    pub layer: String,
    /// Result for this layer.
    pub status: Status,
    /// Layer present in the Python manifest.
    pub in_python: bool,
    /// Layer present in the Rust manifest.
    pub in_rust: bool,
    /// Frames advertised by Python (after the frame filter).
    pub python_frames: usize,
    /// Frames advertised by Rust (after the frame filter).
    pub rust_frames: usize,
    /// Frames advertised by both.
    pub common_frames: usize,
    /// Timestamps only Python advertises (first [`SAMPLE_LIMIT`]).
    pub only_python: Vec<String>,
    /// Timestamps only Rust advertises (first [`SAMPLE_LIMIT`]).
    pub only_rust: Vec<String>,
    /// Count of timestamps only Python advertises.
    pub only_python_count: usize,
    /// Count of timestamps only Rust advertises.
    pub only_rust_count: usize,
    /// `[python, rust]` layer `palettes` when they differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palettes: Option<[Vec<String>; 2]>,
    /// Layer metadata keys only Python writes.
    pub layer_keys_only_python: Vec<String>,
    /// Layer metadata keys only Rust writes.
    pub layer_keys_only_rust: Vec<String>,
    /// Common frames whose entries mismatch (path, palettes or key set).
    pub frame_mismatch_count: usize,
    /// First [`SAMPLE_LIMIT`] frame entries that differ in any way.
    pub frame_diffs: Vec<ManifestFrameDiff>,
}

/// Manifest comparison.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestReport {
    /// Python manifest file, when one was found.
    pub python: Option<String>,
    /// Rust manifest file, when one was found.
    pub rust: Option<String>,
    /// Read or parse failures, and a note when only one side has a manifest.
    pub errors: Vec<String>,
    /// Top-level differences (`schema_version`, templates).
    pub top_level: Vec<String>,
    /// Per-layer differences.
    pub layers: Vec<ManifestLayerDiff>,
    /// Entry mismatches: top-level, missing layers, layer keys and palettes, frame entries.
    /// Frame-list differences are timing and are not counted. These fail only with `--exact`.
    pub mismatches: usize,
}

/// Totals over every compared frame.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    /// Overall result.
    pub status: Status,
    /// Process exit code (0 pass, 1 fail, 2 nothing compared).
    pub exit_code: i32,
    /// Why the run failed or warned.
    pub reasons: Vec<String>,
    /// Pyramids compared.
    pub frames_compared: usize,
    /// Pyramids on the Python side only.
    pub frames_only_python: usize,
    /// Pyramids on the Rust side only.
    pub frames_only_rust: usize,
    /// Frames with status `fail`.
    pub frames_failed: usize,
    /// Frames with status `warn`.
    pub frames_warned: usize,
    /// Tiles on the Python side (compared frames).
    pub python_tiles: usize,
    /// Tiles on the Rust side (compared frames).
    pub rust_tiles: usize,
    /// Tiles on both sides.
    pub common_tiles: usize,
    /// Python-only tiles.
    pub only_python_tiles: usize,
    /// Rust-only tiles.
    pub only_rust_tiles: usize,
    /// Byte-identical common tiles.
    pub identical_tiles: usize,
    /// Visibly identical common tiles.
    pub equivalent_tiles: usize,
    /// Common tiles indexed on both sides.
    pub index_compared_tiles: usize,
    /// Of those, identical palette indices.
    pub index_identical_tiles: usize,
    /// Size-mismatched tiles.
    pub size_mismatch_tiles: usize,
    /// Undecodable tiles.
    pub decode_error_tiles: usize,
    /// Pixels compared.
    pub pixels_compared: u64,
    /// Byte-identical pixels.
    pub pixels_identical: u64,
    /// Visibly differing pixels.
    pub pixels_differing: u64,
    /// `100 - diff_pct`.
    pub identical_pct: Option<f64>,
    /// Visibly differing pixels in percent.
    pub diff_pct: Option<f64>,
    /// Largest channel delta.
    pub max_channel_delta: u8,
    /// Differing pixels by kind.
    pub diff_kinds: DiffKinds,
    /// Non-tile `.png` files skipped (python, rust).
    pub ignored_files: [usize; 2],
}

/// The whole `radar parity` result.
#[derive(Debug, Clone, Serialize)]
pub struct ParityReport {
    /// Python tile root.
    pub python_root: String,
    /// Rust tile root.
    pub rust_root: String,
    /// `exact` or `threshold`.
    pub mode: &'static str,
    /// Per-frame threshold in percent of pixels.
    pub max_diff_pct: f64,
    /// Per pyramid present on both sides, sorted.
    pub frames: Vec<FrameReport>,
    /// Pyramids only on the Python side (first [`SAMPLE_LIMIT`]).
    pub frames_only_python: Vec<String>,
    /// Pyramids only on the Rust side (first [`SAMPLE_LIMIT`]).
    pub frames_only_rust: Vec<String>,
    /// Manifest comparison, when both sides have one.
    pub manifest: Option<ManifestReport>,
    /// Totals and exit status.
    pub summary: Summary,
}

/// Options of [`compare_roots`] other than the roots.
#[derive(Debug, Clone)]
pub struct CompareOptions {
    /// Selection.
    pub filter: Filter,
    /// Per-frame threshold, percent of visibly differing pixels.
    pub max_diff_pct: f64,
    /// Exact mode (`--exact`).
    pub exact: bool,
    /// Python manifest (file or state dir). `None` tries `<python root>/manifest.json`.
    pub python_manifest: Option<PathBuf>,
    /// Rust manifest (file or state dir). `None` tries `<rust root>/manifest.json`.
    pub rust_manifest: Option<PathBuf>,
}

impl CompareOptions {
    /// Options from parsed CLI arguments.
    pub fn from_args(args: &ParityArgs) -> Self {
        Self {
            filter: Filter {
                layers: args.layers.clone(),
                palettes: args.palettes.clone(),
                frames: args.frames.clone(),
                min_zoom: args.min_zoom,
                max_zoom: args.max_zoom,
            },
            max_diff_pct: if args.exact { 0.0 } else { args.max_diff_pct },
            exact: args.exact,
            python_manifest: args.python_manifest.clone(),
            rust_manifest: args.rust_manifest.clone(),
        }
    }
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn sample<I: IntoIterator<Item = String>>(it: I) -> Vec<String> {
    it.into_iter().take(SAMPLE_LIMIT).collect()
}

/// Compare two tile roots. Tile decoding runs on the current rayon pool.
pub fn compare_roots(python: &Path, rust: &Path, opts: &CompareOptions) -> Result<ParityReport> {
    let (py_scan, rs_scan) = rayon::join(
        || scan_tree(python, &opts.filter),
        || scan_tree(rust, &opts.filter),
    );
    let (py_scan, rs_scan) = (py_scan?, rs_scan?);

    let common: Vec<&FrameKey> = py_scan
        .frames
        .keys()
        .filter(|k| rs_scan.frames.contains_key(*k))
        .collect();
    let only_py: Vec<String> = py_scan
        .frames
        .keys()
        .filter(|k| !rs_scan.frames.contains_key(*k))
        .map(ToString::to_string)
        .collect();
    let only_rs: Vec<String> = rs_scan
        .frames
        .keys()
        .filter(|k| !py_scan.frames.contains_key(*k))
        .map(ToString::to_string)
        .collect();

    // One job per tile in the union of each common frame's tile sets.
    let mut jobs = Vec::new();
    for (i, key) in common.iter().enumerate() {
        let a = &py_scan.frames[*key];
        let b = &rs_scan.frames[*key];
        let union: BTreeSet<&TileKey> = a.keys().chain(b.keys()).collect();
        for t in union {
            jobs.push(Job {
                frame: i,
                key: *t,
                python: a.get(t).map(PathBuf::as_path),
                rust: b.get(t).map(PathBuf::as_path),
            });
        }
    }
    let outcomes: Vec<TileOutcome> = jobs.par_iter().map(run_job).collect();

    let mut frames: Vec<FrameReport> = common
        .iter()
        .map(|k| FrameReport {
            layer: k.layer.clone(),
            palette: k.palette.clone(),
            path: k.path.clone(),
            status: Status::Ok,
            problems: Vec::new(),
            python_tiles: py_scan.frames[*k].len(),
            rust_tiles: rs_scan.frames[*k].len(),
            common_tiles: 0,
            only_python_count: 0,
            only_rust_count: 0,
            only_python: Vec::new(),
            only_rust: Vec::new(),
            identical_tiles: 0,
            equivalent_tiles: 0,
            index_compared_tiles: 0,
            index_identical_tiles: 0,
            differing_tiles: Vec::new(),
            size_mismatch: Vec::new(),
            decode_errors: Vec::new(),
            pixels_compared: 0,
            pixels_identical: 0,
            pixels_differing: 0,
            identical_pct: 100.0,
            diff_pct: 0.0,
            max_channel_delta: 0,
            diff_kinds: DiffKinds::default(),
            transparency_only: true,
        })
        .collect();
    let mut stats = vec![PixelStats::default(); frames.len()];
    for o in outcomes {
        let f = &mut frames[o.frame];
        stats[o.frame].add(&o.stats);
        let label = tile_label(o.key);
        match o.side {
            Side::Both {
                rgba_identical,
                index_equal,
            } => {
                f.common_tiles += 1;
                if rgba_identical {
                    f.identical_tiles += 1;
                }
                if o.stats.kinds.visible() == 0 {
                    f.equivalent_tiles += 1;
                } else if f.differing_tiles.len() < SAMPLE_LIMIT {
                    f.differing_tiles.push(label);
                }
                if let Some(eq) = index_equal {
                    f.index_compared_tiles += 1;
                    f.index_identical_tiles += usize::from(eq);
                }
            }
            Side::OnlyPython => {
                f.only_python_count += 1;
                if f.only_python.len() < SAMPLE_LIMIT {
                    f.only_python.push(label);
                }
            }
            Side::OnlyRust => {
                f.only_rust_count += 1;
                if f.only_rust.len() < SAMPLE_LIMIT {
                    f.only_rust.push(label);
                }
            }
            Side::SizeMismatch(msg) => {
                f.common_tiles += 1;
                f.size_mismatch.push(msg);
            }
            Side::DecodeError(msg) => f.decode_errors.push(msg),
        }
    }
    for (f, s) in frames.iter_mut().zip(&stats) {
        finish_frame(f, s, opts);
    }

    let manifest = compare_manifests(python, rust, opts);
    let summary = summarize(
        &frames,
        &stats,
        only_py.len(),
        only_rs.len(),
        [py_scan.ignored_files, rs_scan.ignored_files],
        manifest.as_ref(),
        opts,
    );
    Ok(ParityReport {
        python_root: python.display().to_string(),
        rust_root: rust.display().to_string(),
        mode: if opts.exact { "exact" } else { "threshold" },
        max_diff_pct: opts.max_diff_pct,
        frames,
        frames_only_python: sample(only_py),
        frames_only_rust: sample(only_rs),
        manifest,
        summary,
    })
}

fn finish_frame(f: &mut FrameReport, s: &PixelStats, opts: &CompareOptions) {
    f.pixels_compared = s.pixels;
    f.pixels_identical = s.identical;
    f.pixels_differing = s.kinds.visible();
    f.diff_pct = pct(f.pixels_differing, s.pixels);
    f.identical_pct = 100.0 - f.diff_pct;
    f.max_channel_delta = s.max_channel_delta;
    f.diff_kinds = s.kinds;
    f.transparency_only = s.kinds.color == 0;

    let mut fail = Vec::new();
    let mut warn = Vec::new();
    if !f.decode_errors.is_empty() {
        fail.push(format!("{} undecodable tile(s)", f.decode_errors.len()));
    }
    if !f.size_mismatch.is_empty() {
        fail.push(format!(
            "{} tile(s) with different sizes",
            f.size_mismatch.len()
        ));
    }
    if f.diff_pct > opts.max_diff_pct {
        fail.push(format!(
            "{:.3}% of pixels differ (threshold {}%)",
            f.diff_pct, opts.max_diff_pct
        ));
    } else if f.pixels_differing > 0 {
        warn.push(format!(
            "{:.3}% of pixels differ (within {}%)",
            f.diff_pct, opts.max_diff_pct
        ));
    }
    if f.only_python_count + f.only_rust_count > 0 {
        let msg = format!(
            "tile sets differ: {} only python, {} only rust",
            f.only_python_count, f.only_rust_count
        );
        if opts.exact { &mut fail } else { &mut warn }.push(msg);
    }
    if f.index_identical_tiles < f.index_compared_tiles {
        let msg = format!(
            "palette indices differ in {} of {} indexed tile(s)",
            f.index_compared_tiles - f.index_identical_tiles,
            f.index_compared_tiles
        );
        if opts.exact { &mut fail } else { &mut warn }.push(msg);
    }
    f.status = if !fail.is_empty() {
        Status::Fail
    } else if !warn.is_empty() {
        Status::Warn
    } else {
        Status::Ok
    };
    f.problems = fail.into_iter().chain(warn).collect();
}

fn summarize(
    frames: &[FrameReport],
    stats: &[PixelStats],
    only_py: usize,
    only_rs: usize,
    ignored_files: [usize; 2],
    manifest: Option<&ManifestReport>,
    opts: &CompareOptions,
) -> Summary {
    let mut total = PixelStats::default();
    for s in stats {
        total.add(s);
    }
    let sum = |g: fn(&FrameReport) -> usize| frames.iter().map(g).sum::<usize>();
    let frames_failed = frames.iter().filter(|f| f.status == Status::Fail).count();
    let frames_warned = frames.iter().filter(|f| f.status == Status::Warn).count();
    let diff_pct = (total.pixels > 0).then(|| pct(total.kinds.visible(), total.pixels));

    let mut reasons = Vec::new();
    let mut status = Status::Ok;
    let mut exit_code = 0;
    if frames.is_empty() {
        reasons.push("no frame is present on both sides".to_string());
        status = Status::Fail;
        exit_code = 2;
    }
    if frames_failed > 0 {
        reasons.push(format!("{frames_failed} frame(s) failed"));
        status = Status::Fail;
        exit_code = exit_code.max(1);
    }
    if let Some(m) = manifest {
        if opts.exact && m.mismatches > 0 {
            reasons.push(format!("{} manifest entry mismatch(es)", m.mismatches));
            status = Status::Fail;
            exit_code = exit_code.max(1);
        } else if m.mismatches > 0 || !m.errors.is_empty() {
            reasons.push(format!(
                "manifest: {} entry mismatch(es), {} error(s)",
                m.mismatches,
                m.errors.len()
            ));
            status = status.max(Status::Warn);
        }
    }
    if frames_warned > 0 {
        reasons.push(format!(
            "{frames_warned} frame(s) differ within the threshold"
        ));
        status = status.max(Status::Warn);
    }
    if only_py + only_rs > 0 {
        reasons.push(format!(
            "frames on one side only: {only_py} python, {only_rs} rust"
        ));
        status = status.max(Status::Warn);
    }
    Summary {
        status,
        exit_code,
        reasons,
        frames_compared: frames.len(),
        frames_only_python: only_py,
        frames_only_rust: only_rs,
        frames_failed,
        frames_warned,
        python_tiles: sum(|f| f.python_tiles),
        rust_tiles: sum(|f| f.rust_tiles),
        common_tiles: sum(|f| f.common_tiles),
        only_python_tiles: sum(|f| f.only_python_count),
        only_rust_tiles: sum(|f| f.only_rust_count),
        identical_tiles: sum(|f| f.identical_tiles),
        equivalent_tiles: sum(|f| f.equivalent_tiles),
        index_compared_tiles: sum(|f| f.index_compared_tiles),
        index_identical_tiles: sum(|f| f.index_identical_tiles),
        size_mismatch_tiles: sum(|f| f.size_mismatch.len()),
        decode_error_tiles: sum(|f| f.decode_errors.len()),
        pixels_compared: total.pixels,
        pixels_identical: total.identical,
        pixels_differing: total.kinds.visible(),
        identical_pct: diff_pct.map(|d| 100.0 - d),
        diff_pct,
        max_channel_delta: total.max_channel_delta,
        diff_kinds: total.kinds,
        ignored_files,
    }
}

// ------------------------------------------------------------------------------------------------
// Manifests
// ------------------------------------------------------------------------------------------------

fn manifest_file(explicit: Option<&Path>, root: &Path) -> Option<PathBuf> {
    match explicit {
        Some(p) if p.is_dir() => Some(radar_core::manifest::manifest_path(p)),
        Some(p) => Some(p.to_path_buf()),
        None => {
            let p = radar_core::manifest::manifest_path(root);
            p.is_file().then_some(p)
        }
    }
}

fn read_manifest_file(path: &Path) -> Result<radar_core::manifest::Manifest> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    radar_core::manifest::parse_manifest(&text).with_context(|| format!("parse {}", path.display()))
}

fn key_diff(a: &Map<String, Value>, b: &Map<String, Value>) -> (Vec<String>, Vec<String>) {
    (
        a.keys().filter(|k| !b.contains_key(*k)).cloned().collect(),
        b.keys().filter(|k| !a.contains_key(*k)).cloned().collect(),
    )
}

fn compare_manifests(python: &Path, rust: &Path, opts: &CompareOptions) -> Option<ManifestReport> {
    let py_file = manifest_file(opts.python_manifest.as_deref(), python);
    let rs_file = manifest_file(opts.rust_manifest.as_deref(), rust);
    if py_file.is_none() && rs_file.is_none() {
        return None;
    }
    let mut report = ManifestReport {
        python: py_file.as_ref().map(|p| p.display().to_string()),
        rust: rs_file.as_ref().map(|p| p.display().to_string()),
        errors: Vec::new(),
        top_level: Vec::new(),
        layers: Vec::new(),
        mismatches: 0,
    };
    let mut read = |side: &str, file: Option<&PathBuf>| match file {
        None => {
            report
                .errors
                .push(format!("no {side} manifest; manifests not compared"));
            None
        }
        Some(p) => match read_manifest_file(p) {
            Ok(m) => Some(m),
            Err(e) => {
                report.errors.push(format!("{side}: {e:#}"));
                None
            }
        },
    };
    let py = read("python", py_file.as_ref());
    let rs = read("rust", rs_file.as_ref());
    let (Some(py), Some(rs)) = (py, rs) else {
        return Some(report);
    };

    if py.schema_version != rs.schema_version {
        report.top_level.push(format!(
            "schema_version: python {}, rust {}",
            py.schema_version, rs.schema_version
        ));
    }
    if py.tile_url_template != rs.tile_url_template {
        report.top_level.push(format!(
            "tile_url_template: python {:?}, rust {:?}",
            py.tile_url_template, rs.tile_url_template
        ));
    }
    if py.tile_url_template_legacy != rs.tile_url_template_legacy {
        report.top_level.push(format!(
            "tile_url_template_legacy: python {:?}, rust {:?}",
            py.tile_url_template_legacy, rs.tile_url_template_legacy
        ));
    }
    let (top_py, top_rs) = key_diff(&py.extra, &rs.extra);
    for k in top_py {
        report
            .top_level
            .push(format!("top-level key only in python: {k}"));
    }
    for k in top_rs {
        report
            .top_level
            .push(format!("top-level key only in rust: {k}"));
    }
    report.mismatches += report.top_level.len();

    let names: BTreeSet<&String> = py.layers.keys().chain(rs.layers.keys()).collect();
    for name in names {
        if !opts.filter.layer_ok(name) {
            continue;
        }
        let diff = compare_layer(name, py.layers.get(name), rs.layers.get(name), &opts.filter);
        report.mismatches += layer_mismatches(&diff);
        report.layers.push(diff);
    }
    Some(report)
}

fn layer_mismatches(d: &ManifestLayerDiff) -> usize {
    if !d.in_python || !d.in_rust {
        return 1;
    }
    usize::from(d.palettes.is_some())
        + d.layer_keys_only_python.len()
        + d.layer_keys_only_rust.len()
        + d.frame_mismatch_count
}

fn compare_layer(
    name: &str,
    py: Option<&radar_core::manifest::Layer>,
    rs: Option<&radar_core::manifest::Layer>,
    filter: &Filter,
) -> ManifestLayerDiff {
    let pick =
        |l: Option<&radar_core::manifest::Layer>| -> BTreeMap<String, radar_core::manifest::Frame> {
            l.map(|l| {
                l.frames
                    .iter()
                    .filter(|f| filter.frame_matches(&f.path) || filter.frame_matches(&f.timestamp))
                    .map(|f| (f.timestamp.clone(), f.clone()))
                    .collect()
            })
            .unwrap_or_default()
        };
    let (pf, rf) = (pick(py), pick(rs));
    let only_py: Vec<String> = pf
        .keys()
        .filter(|t| !rf.contains_key(*t))
        .cloned()
        .collect();
    let only_rs: Vec<String> = rf
        .keys()
        .filter(|t| !pf.contains_key(*t))
        .cloned()
        .collect();
    let mut frame_diffs = Vec::new();
    let mut frame_mismatch_count = 0;
    let mut common = 0;
    for (ts, a) in &pf {
        let Some(b) = rf.get(ts) else { continue };
        common += 1;
        let (kp, kr) = key_diff(&a.extra, &b.extra);
        let values_differ = a
            .extra
            .iter()
            .filter(|(k, v)| b.extra.get(*k).is_some_and(|w| w != *v))
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>();
        let d = ManifestFrameDiff {
            timestamp: ts.clone(),
            path: (a.path != b.path).then(|| [a.path.clone(), b.path.clone()]),
            palettes: (a.palettes != b.palettes).then(|| [a.palettes.clone(), b.palettes.clone()]),
            keys_only_python: kp,
            keys_only_rust: kr,
            values_differ,
        };
        if d.is_mismatch() {
            frame_mismatch_count += 1;
        }
        if (d.is_mismatch() || !d.values_differ.is_empty()) && frame_diffs.len() < SAMPLE_LIMIT {
            frame_diffs.push(d);
        }
    }
    let (lkp, lkr) = match (py, rs) {
        (Some(a), Some(b)) => key_diff(&a.extra, &b.extra),
        _ => (Vec::new(), Vec::new()),
    };
    let palettes = match (py, rs) {
        (Some(a), Some(b)) if a.palettes != b.palettes => {
            Some([a.palettes.clone(), b.palettes.clone()])
        }
        _ => None,
    };
    let mut d = ManifestLayerDiff {
        layer: name.to_string(),
        status: Status::Ok,
        in_python: py.is_some(),
        in_rust: rs.is_some(),
        python_frames: pf.len(),
        rust_frames: rf.len(),
        common_frames: common,
        only_python_count: only_py.len(),
        only_rust_count: only_rs.len(),
        only_python: sample(only_py),
        only_rust: sample(only_rs),
        palettes,
        layer_keys_only_python: lkp,
        layer_keys_only_rust: lkr,
        frame_mismatch_count,
        frame_diffs,
    };
    let differs = layer_mismatches(&d) > 0
        || d.only_python_count + d.only_rust_count > 0
        || !d.frame_diffs.is_empty();
    d.status = if differs { Status::Warn } else { Status::Ok };
    d
}

// ------------------------------------------------------------------------------------------------
// Output and entry point
// ------------------------------------------------------------------------------------------------

/// Render the human-readable report.
pub fn render_human(r: &ParityReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "== parity — python {} vs rust {} ==",
        r.python_root, r.rust_root
    );
    if r.mode == "exact" {
        let _ = writeln!(
            out,
            "mode: exact (Python indexed renderer: identical tiles, indices and pixels expected)"
        );
    } else {
        let _ = writeln!(
            out,
            "mode: threshold {}% differing pixels per frame (Python legacy RGBA vs Rust indexed is ~92% identical by design)",
            r.max_diff_pct
        );
    }
    for f in &r.frames {
        let _ = write!(
            out,
            "  {} {}/{}/{}: tiles {}/{} ({} common, {} identical",
            f.status.glyph(),
            f.layer,
            f.palette,
            f.path,
            f.python_tiles,
            f.rust_tiles,
            f.common_tiles,
            f.identical_tiles
        );
        if f.index_compared_tiles > 0 {
            let _ = write!(
                out,
                ", indices equal {}/{}",
                f.index_identical_tiles, f.index_compared_tiles
            );
        }
        let _ = write!(out, "); pixels {:.3}% identical", f.identical_pct);
        if f.pixels_differing > 0 {
            let edge = f.diff_kinds.alpha_edge + f.diff_kinds.alpha_only;
            let _ = write!(
                out,
                ", {} differ ({:.0}% transparency edges, {:.0}% colour), max Δ {}",
                f.pixels_differing,
                pct(edge, f.pixels_differing),
                pct(f.diff_kinds.color, f.pixels_differing),
                f.max_channel_delta
            );
        }
        let _ = writeln!(out);
        for p in &f.problems {
            let _ = writeln!(out, "      - {p}");
        }
        for (label, list, count) in [
            ("only python", &f.only_python, f.only_python_count),
            ("only rust", &f.only_rust, f.only_rust_count),
        ] {
            if count > 0 {
                let _ = writeln!(
                    out,
                    "      {label}: {}{}",
                    list.join(", "),
                    more(count, list.len())
                );
            }
        }
        for e in f.decode_errors.iter().chain(&f.size_mismatch) {
            let _ = writeln!(out, "      ! {e}");
        }
    }
    let s = &r.summary;
    for (label, list, count) in [
        (
            "frames only in python",
            &r.frames_only_python,
            s.frames_only_python,
        ),
        (
            "frames only in rust",
            &r.frames_only_rust,
            s.frames_only_rust,
        ),
    ] {
        if count > 0 {
            let _ = writeln!(
                out,
                "  ⚠ {label} ({count}): {}{}",
                list.join(", "),
                more(count, list.len())
            );
        }
    }
    if let Some(m) = &r.manifest {
        let _ = writeln!(out, "== manifest ==");
        for e in &m.errors {
            let _ = writeln!(out, "  ⚠ {e}");
        }
        for t in &m.top_level {
            let _ = writeln!(out, "  ⚠ {t}");
        }
        for l in &m.layers {
            let presence = match (l.in_python, l.in_rust) {
                (true, true) => String::new(),
                (true, false) => " (python only)".into(),
                (false, true) => " (rust only)".into(),
                (false, false) => String::new(),
            };
            let _ = writeln!(
                out,
                "  {} {}{}: frames {}/{} ({} common, {} only python, {} only rust), {} entry mismatch(es)",
                l.status.glyph(),
                l.layer,
                presence,
                l.python_frames,
                l.rust_frames,
                l.common_frames,
                l.only_python_count,
                l.only_rust_count,
                l.frame_mismatch_count
            );
            if let Some([a, b]) = &l.palettes {
                let _ = writeln!(out, "      palettes: python {a:?}, rust {b:?}");
            }
            if !l.layer_keys_only_python.is_empty() || !l.layer_keys_only_rust.is_empty() {
                let _ = writeln!(
                    out,
                    "      layer keys: only python {:?}, only rust {:?}",
                    l.layer_keys_only_python, l.layer_keys_only_rust
                );
            }
            for d in &l.frame_diffs {
                let mut parts = Vec::new();
                if let Some([a, b]) = &d.path {
                    parts.push(format!("path {a:?} vs {b:?}"));
                }
                if let Some([a, b]) = &d.palettes {
                    parts.push(format!("palettes {a:?} vs {b:?}"));
                }
                if !d.keys_only_python.is_empty() {
                    parts.push(format!("keys only python {:?}", d.keys_only_python));
                }
                if !d.keys_only_rust.is_empty() {
                    parts.push(format!("keys only rust {:?}", d.keys_only_rust));
                }
                if !d.values_differ.is_empty() {
                    parts.push(format!("values differ {:?}", d.values_differ));
                }
                let _ = writeln!(out, "      {}: {}", d.timestamp, parts.join("; "));
            }
        }
    }
    let _ = writeln!(out, "== summary ==");
    let _ = writeln!(
        out,
        "{} frames · {} fail · {} warn · tiles {}/{} ({} identical, {} visibly identical) · pixels {} identical · max Δ {}",
        s.frames_compared,
        s.frames_failed,
        s.frames_warned,
        s.python_tiles,
        s.rust_tiles,
        s.identical_tiles,
        s.equivalent_tiles,
        s.identical_pct
            .map_or("n/a".to_string(), |p| format!("{p:.3}%")),
        s.max_channel_delta
    );
    let _ = writeln!(
        out,
        "{} {} (exit {}){}",
        s.status.glyph(),
        match s.status {
            Status::Ok => "PASS",
            Status::Warn => "PASS with differences",
            Status::Fail => "FAIL",
        },
        s.exit_code,
        if s.reasons.is_empty() {
            String::new()
        } else {
            format!(": {}", s.reasons.join("; "))
        }
    );
    out
}

fn more(count: usize, shown: usize) -> String {
    if count > shown {
        format!(" … (+{})", count - shown)
    } else {
        String::new()
    }
}

/// `radar parity`: compare, print, and return the exit code.
pub fn run(args: &ParityArgs) -> Result<i32> {
    if args.max_diff_pct.is_nan() || args.max_diff_pct < 0.0 {
        bail!("--max-diff-pct must be a percentage >= 0");
    }
    if let (Some(lo), Some(hi)) = (args.min_zoom, args.max_zoom)
        && lo > hi
    {
        bail!("--min-zoom {lo} is above --max-zoom {hi}");
    }
    let opts = CompareOptions::from_args(args);
    let compare = || compare_roots(&args.python_root, &args.rust_root, &opts);
    let report = match args.threads {
        Some(n) => rayon::ThreadPoolBuilder::new()
            .num_threads(n.max(1))
            .build()
            .context("build decoder thread pool")?
            .install(compare)?,
        None => compare()?,
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_human(&report));
    }
    Ok(report.summary.exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct Wrap {
        #[command(flatten)]
        args: ParityArgs,
    }

    #[test]
    fn cli_defaults_and_flags() {
        let w = Wrap::try_parse_from(["p", "--python-root", "/a", "--rust-root", "/b"]).unwrap();
        assert_eq!(w.args.max_diff_pct, DEFAULT_MAX_DIFF_PCT);
        assert!(!w.args.exact && !w.args.json);
        let w = Wrap::try_parse_from([
            "p",
            "--python-root",
            "/a",
            "--rust-root",
            "/b",
            "--product",
            "radar",
            "--layer",
            "nowcast",
            "--palette",
            "classic",
            "--frame",
            "2026-10-14T06:00:00Z",
            "--min-zoom",
            "4",
            "--max-zoom",
            "6",
            "--exact",
            "--json",
            "--threads",
            "2",
        ])
        .unwrap();
        let o = CompareOptions::from_args(&w.args);
        assert_eq!(o.filter.layers, ["radar", "nowcast"]);
        assert_eq!(o.max_diff_pct, 0.0);
        assert!(o.exact);
        assert_eq!((o.filter.min_zoom, o.filter.max_zoom), (Some(4), Some(6)));
    }

    #[test]
    fn exact_conflicts_with_explicit_threshold() {
        assert!(
            Wrap::try_parse_from([
                "p",
                "--python-root",
                "/a",
                "--rust-root",
                "/b",
                "--exact",
                "--max-diff-pct",
                "5"
            ])
            .is_err()
        );
    }

    #[test]
    fn tile_path_split() {
        assert_eq!(
            split_tile_path(&["2026-01-01T00:00:00+00:00", "4", "3", "5.png"]),
            Some(("2026-01-01T00:00:00+00:00".into(), 4, 3, 5))
        );
        assert_eq!(
            split_tile_path(&["runs", "20260715_12", "t", "7", "1", "2.png"]),
            Some(("runs/20260715_12/t".into(), 7, 1, 2))
        );
        assert_eq!(split_tile_path(&["4", "3", "5.png"]), None);
        assert_eq!(split_tile_path(&["f", "a", "3", "5.png"]), None);
    }

    #[test]
    fn frame_filter_matches_suffix_and_canonical_timestamp() {
        let f = Filter {
            frames: vec!["2026-07-15T13:00:00Z".into()],
            ..Filter::default()
        };
        assert!(f.frame_matches("2026-07-15T13:00:00+00:00"));
        assert!(f.frame_matches("runs/20260715_12/2026-07-15T13:00:00+00:00"));
        assert!(!f.frame_matches("2026-07-15T14:00:00+00:00"));
        assert!(Filter::default().frame_matches("anything"));
    }

    #[test]
    fn pixel_kinds() {
        let a = [
            1, 2, 3, 255, // identical
            9, 9, 9, 0, // both transparent, rgb differs
            10, 20, 30, 255, // edge: transparent on b
            10, 20, 30, 255, // alpha only
            10, 20, 30, 255, // colour
        ];
        let b = [
            1, 2, 3, 255, //
            0, 0, 0, 0, //
            0, 0, 0, 0, //
            10, 20, 30, 128, //
            10, 25, 30, 255, //
        ];
        let s = compare_rgba(&a, &b);
        assert_eq!(s.pixels, 5);
        assert_eq!(s.identical, 1);
        assert_eq!(
            s.kinds,
            DiffKinds {
                transparent_rgb_only: 1,
                alpha_edge: 1,
                alpha_only: 1,
                color: 1
            }
        );
        assert_eq!(s.kinds.visible(), 3);
        assert_eq!(s.max_channel_delta, 255);
    }

    #[test]
    fn rejects_non_png() {
        assert!(decode_rgba(b"hello").is_err());
        assert!(decode_rgba(&[0u8; 64]).is_err());
    }
}
