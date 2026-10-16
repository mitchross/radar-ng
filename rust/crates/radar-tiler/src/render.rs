//! Frame rendering entry point (spec-tiler sections 9.2 to 9.5).
//!
//! [`render_frame_palettes`] is the production path: it builds one shared class model over the
//! frame's palettes, computes the per-palette identities, and publishes one pyramid per palette
//! through the atomic protocol in [`crate::publish`]. Each tile is sampled and classified once,
//! encoded once (IDAT and IEND), and written per palette as `header + PLTE/tRNS + shared tail`.
//! Tiles run in parallel with rayon; every worker owns one [`Scratch`] buffer set.

use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use radar_geo::{Grid, GridGeo};
use rayon::prelude::*;

use crate::cache::{GeometryKey, global_cache};
use crate::classify::{ClassModel, PaletteLut};
use crate::error::{Result, TilerError};
use crate::geometry::{AxisMap, MAX_ZOOM, Sampling, SourceGeometry, TileGeometry};
use crate::identity::{
    FrameKind, GridIdentity, array_digest_f32, frame_contract, frame_identities, frame_model,
    grid_spec_digest,
};
use crate::palettes::{ColorTable, validate_palette_name};
use crate::png::{IndexedTileEncoder, append_palette_chunks, bit_depth_for_entries, header_bytes};
use crate::publish::{MultiPaletteRenderResult, publish_palette_set};
use crate::sample::{
    footprint_edges, max_block_factors, reduce_column_ranges, reduce_row_ranges,
    region_transparent, sample_bilinear_into, sample_nearest_into,
};

/// Overview modes (spec 4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overview {
    /// Max-pool the source footprint of each output pixel, for zooms where source cells are
    /// finer than one output pixel. Native zooms are sampled as usual.
    Max,
}

impl Overview {
    fn as_str(self) -> &'static str {
        match self {
            Overview::Max => "max",
        }
    }
}

/// Options for [`render_frame_palettes`].
#[derive(Debug, Clone)]
pub struct FrameOptions {
    /// Tile edge in pixels (spec default 256).
    pub tile_size: usize,
    /// Stable frame identity. Must be non-empty.
    pub source_id: String,
    /// Publish lock root. `None` uses each final's grandparent directory (spec 13.1).
    pub publication_lock_root: Option<PathBuf>,
    /// Overview mode, or `None` for native sampling only.
    pub overview: Option<Overview>,
    /// zlib level for IDAT, `0..=9`. Defaults to `TILE_PNG_COMPRESS_LEVEL`.
    pub png_compress_level: u32,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            tile_size: 256,
            source_id: "unspecified-frame".to_string(),
            publication_lock_root: None,
            overview: None,
            png_compress_level: crate::env::png_compress_level(),
        }
    }
}

/// Render one frame into one published pyramid per palette (spec 9.5 and 13).
///
/// * `grid` holds the frame values (`f32`, NaN = missing) and its coordinates. Categorical
///   frames store their integer ids as `f32`.
/// * `kind` selects continuous (bilinear, then classify) or categorical (nearest) rendering.
/// * `palettes` supplies a color table for every entry of `output_dirs`. Extra entries are
///   ignored.
/// * `output_dirs` pairs palette names with their final pyramid roots. Results come back in
///   this order.
///
/// Palettes whose final already holds a complete pyramid are not re-rendered. A complete
/// pyramid with a different identity is reported as [`crate::PublishStatus::ExistingForeign`]
/// and left untouched.
///
/// Holds the publication `flock` while it renders tiles on the current Rayon pool. Never call it
/// from several parallel tasks of one Rayon pool: a worker blocked in the lock can be the one a
/// lock holder needs to finish its tiles, and both wait forever (the 2026-10-14 nowcast deadlock).
/// Call it from a plain thread, or one frame at a time inside a pool.
pub fn render_frame_palettes(
    grid: &Grid,
    kind: &FrameKind,
    palettes: &[(String, ColorTable)],
    output_dirs: &[(String, PathBuf)],
    zoom_levels: &[u32],
    options: &FrameOptions,
) -> Result<MultiPaletteRenderResult> {
    if output_dirs.is_empty() {
        return Ok(MultiPaletteRenderResult {
            outcomes: Vec::new(),
        });
    }
    validate_options(options, zoom_levels)?;
    validate_kind(kind)?;
    if grid.height == 0 || grid.width == 0 || grid.values.len() != grid.height * grid.width {
        return Err(TilerError::Invalid(
            "grid values must be height x width with at least one cell".into(),
        ));
    }
    let mut tables: Vec<(String, ColorTable)> = Vec::with_capacity(output_dirs.len());
    for (name, _) in output_dirs {
        validate_palette_name(name)?;
        let (_, table) = palettes.iter().find(|(n, _)| n == name).ok_or_else(|| {
            TilerError::Invalid(format!("missing color table for palette {name}"))
        })?;
        tables.push((name.clone(), table.clone()));
    }
    let model = frame_model(kind, &tables)?;
    if options.overview.is_some() {
        if matches!(kind, FrameKind::Categorical { .. }) {
            return Err(TilerError::Invalid(
                "overview is not supported for categorical frames".into(),
            ));
        }
        if !matches!(grid.geo, GridGeo::Regular { .. }) {
            return Err(TilerError::Invalid(
                "overview requires regular 1-D lats/lons".into(),
            ));
        }
    }
    let overview = options.overview.map(Overview::as_str);
    let src = SourceGeometry::from_grid(grid)?;
    let contract = frame_contract(kind, &model, options.png_compress_level, overview);
    // Categorical frames hash their f32 storage here, not the caller's integer dtype (README).
    let source_digest = array_digest_f32(&grid.values, &[grid.height, grid.width]);
    let grid_digest = grid_spec_digest(src.identity());
    let identities = frame_identities(
        &options.source_id,
        &source_digest,
        &grid_digest,
        zoom_levels,
        options.tile_size,
        &contract,
        &tables,
    )?;
    let job = FrameJob::new(grid, src, kind, &model, options, zoom_levels)?;
    publish_palette_set(
        output_dirs,
        &identities,
        options.publication_lock_root.as_deref(),
        |staged| job.render(staged),
    )
}

fn validate_options(options: &FrameOptions, zooms: &[u32]) -> Result<()> {
    if options.tile_size == 0 || options.tile_size > 4096 {
        return Err(TilerError::Invalid("tile_size must be in 1..=4096".into()));
    }
    if options.source_id.is_empty() {
        return Err(TilerError::Invalid(
            "source_id must be a non-empty stable frame identity".into(),
        ));
    }
    if options.png_compress_level > 9 {
        return Err(TilerError::Invalid(
            "png_compress_level must be in 0..=9".into(),
        ));
    }
    if zooms.iter().any(|&z| z > MAX_ZOOM) {
        return Err(TilerError::Invalid(format!(
            "zoom levels must be in 0..={MAX_ZOOM}"
        )));
    }
    Ok(())
}

fn validate_kind(kind: &FrameKind) -> Result<()> {
    if let FrameKind::Continuous {
        min_valid_weight, ..
    } = kind
        && !(*min_valid_weight > 0.0 && *min_valid_weight <= 1.0)
    {
        return Err(TilerError::Invalid(
            "min_valid_weight must be in (0, 1]".into(),
        ));
    }
    Ok(())
}

/// How one tile is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// Categorical: nearest gather from the whole-grid class index.
    Nearest,
    /// Continuous: bilinear physical sampling, then classify.
    Bilinear,
    /// Overview: max-pooled footprint, then classify.
    Footprint,
}

#[derive(Debug, Clone, Copy)]
struct TileJob {
    z: u32,
    tx: u32,
    ty: u32,
    route: Route,
}

/// Overview state, built only when `overview` is set.
struct PoolCtx {
    lat_axis: AxisMap,
    lon_axis: AxisMap,
    /// Frame values with `f32(nodata)` replaced by NaN (spec 4.4 step 1). `None` when the frame has
    /// no nodata value, so the grid values are used as they are.
    nan_values: Option<Vec<f32>>,
}

/// Per-frame sampling state: tile routing, geometry and the physical-value kernels. Shared by the
/// indexed renderer ([`FrameJob`]) and the packed-value sampler (`crate::packed`). Read-only after
/// construction.
pub(crate) struct Sampler<'a> {
    grid: &'a Grid,
    src: SourceGeometry,
    identity: Arc<GridIdentity>,
    tile: usize,
    nodata: Option<f64>,
    min_valid_weight: f64,
    pool: Option<PoolCtx>,
    tiles: Vec<TileJob>,
}

impl<'a> Sampler<'a> {
    /// Plan the tiles of `zooms`. `overview` enables the max-pooled route on pooled zooms; it needs a
    /// regular grid. Categorical frames always take the nearest route.
    #[allow(clippy::too_many_arguments)] // internal constructor shared by two callers
    pub(crate) fn new(
        grid: &'a Grid,
        src: SourceGeometry,
        nodata: Option<f64>,
        min_valid_weight: f64,
        categorical: bool,
        overview: bool,
        tile: usize,
        zooms: &[u32],
    ) -> Result<Self> {
        let regular = match &grid.geo {
            GridGeo::Regular { lats, lons } => Some((lats, lons)),
            GridGeo::Projected { .. } => None,
        };
        let mut pool = None;
        if overview {
            let (lats, lons) = regular.ok_or_else(|| {
                TilerError::Invalid("overview requires regular 1-D lats/lons".into())
            })?;
            if lats.len() < 2 || lons.len() < 2 {
                return Err(TilerError::Invalid(
                    "overview requires at least two rows and two columns".into(),
                ));
            }
            let nan_values = nodata.map(|nd| {
                grid.values
                    .iter()
                    .map(|&v| if v == nd as f32 { f32::NAN } else { v })
                    .collect()
            });
            pool = Some(PoolCtx {
                lat_axis: AxisMap::new(lats)?,
                lon_axis: AxisMap::new(lons)?,
                nan_values,
            });
        }
        let mut tiles = Vec::new();
        for &z in zooms {
            let pooled = match regular {
                Some((lats, lons)) if pool.is_some() => {
                    max_block_factors(lats, lons, z, tile) != (1, 1)
                }
                _ => false,
            };
            let route = if categorical {
                Route::Nearest
            } else if pooled {
                Route::Footprint
            } else {
                Route::Bilinear
            };
            for (z, tx, ty) in src.tile_jobs(&[z]) {
                tiles.push(TileJob { z, tx, ty, route });
            }
        }
        let identity = Arc::new(src.identity().clone());
        Ok(Self {
            grid,
            src,
            identity,
            tile,
            nodata,
            min_valid_weight,
            pool,
            tiles,
        })
    }

    /// Tile edge in pixels.
    pub(crate) fn tile_size(&self) -> usize {
        self.tile
    }

    /// `(z, x, y)` of every planned tile, in spec order.
    pub(crate) fn tile_coords(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        self.tiles.iter().map(|j| (j.z, j.tx, j.ty))
    }

    fn geometry(&self, sampling: Sampling, job: TileJob) -> Result<Option<Arc<TileGeometry>>> {
        let key = GeometryKey {
            grid: Arc::clone(&self.identity),
            sampling,
            tile_size: self.tile,
            z: job.z,
            tx: job.tx,
            ty: job.ty,
        };
        global_cache().get_or_compute(key, || {
            self.src
                .tile_geometry(sampling, self.tile, job.z, job.tx, job.ty)
        })
    }

    /// Physical values of planned tile `index` into `out` (`tile * tile`, NaN = no value): bilinear
    /// at native zooms, max-pooled footprint at pooled zooms. Returns `false`, with `out` unspecified,
    /// when the tile has no source coverage. Categorical tiles are an error.
    pub(crate) fn values_at(&self, index: usize, out: &mut Vec<f32>) -> Result<bool> {
        let job = *self
            .tiles
            .get(index)
            .ok_or_else(|| TilerError::Invalid(format!("tile index {index} out of range")))?;
        self.values_into(job, out)
    }

    fn values_into(&self, job: TileJob, out: &mut Vec<f32>) -> Result<bool> {
        match job.route {
            Route::Nearest => Err(TilerError::Invalid(
                "categorical tiles have no physical values".into(),
            )),
            Route::Bilinear => {
                let Some(geom) = self.geometry(Sampling::Bilinear, job)? else {
                    return Ok(false);
                };
                out.resize(self.tile * self.tile, f32::NAN);
                sample_bilinear_into(
                    &geom,
                    &self.grid.values,
                    self.grid.height,
                    self.grid.width,
                    self.tile,
                    self.nodata,
                    self.min_valid_weight,
                    out,
                )?;
                Ok(true)
            }
            Route::Footprint => {
                let pool = self.pool.as_ref().ok_or_else(|| {
                    TilerError::Corrupt("footprint route without overview state".into())
                })?;
                self.footprint_values(pool, job, out)
            }
        }
    }

    /// Max-pooled footprint values of a tile (spec 4.4). `false` when no source row is covered.
    fn footprint_values(&self, pool: &PoolCtx, job: TileJob, out: &mut Vec<f32>) -> Result<bool> {
        let t = self.tile;
        let tf = t as f64;
        let n = f64::from(1u32 << job.z);
        let (tx, ty) = (f64::from(job.tx), f64::from(job.ty));
        let col_edges: Vec<f64> = (0..=t)
            .map(|j| (tx + j as f64 / tf) / n * 360.0 - 180.0)
            .collect();
        let lat_edges: Vec<f64> = (0..=t)
            .map(|i| {
                let y = 1.0 - 2.0 * (ty + i as f64 / tf) / n;
                (PI * y).sinh().atan() * (180.0 / PI)
            })
            .collect();
        let row_ranges = footprint_edges(&lat_edges, &pool.lat_axis);
        let col_ranges = footprint_edges(&col_edges, &pool.lon_axis);
        let (mut rmin, mut rmax) = (usize::MAX, 0usize);
        for &(a, b) in &row_ranges {
            if b > a {
                rmin = rmin.min(a);
                rmax = rmax.max(b);
            }
        }
        if rmin == usize::MAX {
            return Ok(false);
        }
        let w = self.grid.width;
        let values = pool.nan_values.as_deref().unwrap_or(&self.grid.values);
        // Only source rows inside the union of row footprints feed the tile.
        let rows = &values[rmin * w..rmax * w];
        let shifted: Vec<(usize, usize)> = row_ranges
            .iter()
            .map(|&(a, b)| if b > a { (a - rmin, b - rmin) } else { (0, 0) })
            .collect();
        let cols = reduce_column_ranges(rows, rmax - rmin, w, &col_ranges);
        *out = reduce_row_ranges(&cols, col_ranges.len(), &shifted);
        Ok(true)
    }
}

/// Per-frame state shared by every tile worker. Read-only after construction.
struct FrameJob<'a> {
    sampler: Sampler<'a>,
    model: &'a ClassModel,
    level: u32,
    depth: u8,
    /// Whole-grid class index for categorical frames; empty otherwise.
    class_index: Vec<u8>,
}

/// Per-worker buffers, reused across tiles.
struct Scratch {
    values: Vec<f32>,
    classes: Vec<u8>,
    encoder: IndexedTileEncoder,
    tail: Vec<u8>,
    file: Vec<u8>,
}

impl Scratch {
    fn new(tile: usize, level: u32) -> Self {
        Self {
            values: vec![0.0; tile * tile],
            classes: vec![0; tile * tile],
            encoder: IndexedTileEncoder::new(level),
            tail: Vec::new(),
            file: Vec::new(),
        }
    }
}

impl<'a> FrameJob<'a> {
    fn new(
        grid: &'a Grid,
        src: SourceGeometry,
        kind: &FrameKind,
        model: &'a ClassModel,
        options: &FrameOptions,
        zooms: &[u32],
    ) -> Result<Self> {
        let (nodata, min_valid_weight, categorical) = match kind {
            FrameKind::Continuous {
                nodata_value,
                min_valid_weight,
            } => (*nodata_value, *min_valid_weight, false),
            FrameKind::Categorical { .. } => (None, 1.0, true),
        };
        let mut class_index = Vec::new();
        if categorical {
            class_index = vec![0u8; grid.height * grid.width];
            model.classify_category_into(&grid.values, &mut class_index)?;
        }
        let sampler = Sampler::new(
            grid,
            src,
            nodata,
            min_valid_weight,
            categorical,
            options.overview.is_some(),
            options.tile_size,
            zooms,
        )?;
        Ok(Self {
            sampler,
            model,
            level: options.png_compress_level,
            depth: bit_depth_for_entries(model.n_classes()),
            class_index,
        })
    }

    /// Render every tile for the staged palettes. Returns one count per palette; all are equal.
    fn render(&self, staged: &[(String, PathBuf)]) -> Result<Vec<u64>> {
        let mut palettes: Vec<(&[u8], &[u8])> = Vec::with_capacity(staged.len());
        for (name, _) in staged {
            let lut: &PaletteLut = self.model.lut(name).ok_or_else(|| {
                TilerError::Invalid(format!("missing lookup table for palette {name}"))
            })?;
            palettes.push((lut.plte(), lut.trns()));
        }
        let roots: Vec<&Path> = staged.iter().map(|(_, p)| p.as_path()).collect();
        let (tile, level) = (self.sampler.tile, self.level);
        let count = self
            .sampler
            .tiles
            .par_iter()
            .map_init(
                || Scratch::new(tile, level),
                |s, job| self.render_tile(s, *job, &roots, &palettes),
            )
            .try_reduce(|| 0u64, |a, b| Ok(a + b))?;
        Ok(vec![count; staged.len()])
    }

    fn render_tile(
        &self,
        s: &mut Scratch,
        job: TileJob,
        roots: &[&Path],
        palettes: &[(&[u8], &[u8])],
    ) -> Result<u64> {
        let sampler = &self.sampler;
        match job.route {
            Route::Nearest => {
                let Some(geom) = sampler.geometry(Sampling::Nearest, job)? else {
                    return Ok(0);
                };
                let w = sampler.grid.width;
                if region_transparent(&self.class_index, w, geom.row_bounds, geom.col_bounds) {
                    return Ok(0);
                }
                sample_nearest_into(&geom, &self.class_index, w, sampler.tile, &mut s.classes)?;
            }
            Route::Bilinear | Route::Footprint => {
                if !sampler.values_into(job, &mut s.values)? {
                    return Ok(0);
                }
                self.model.classify_f32_into(&s.values, &mut s.classes)?;
            }
        }
        if !s.classes.iter().any(|&c| c != 0) {
            return Ok(0);
        }
        self.write_tile(s, job, roots, palettes)?;
        Ok(1)
    }

    /// Encode the tile once and write it under every staged palette (spec 9.2, `_write_index_tile`).
    fn write_tile(
        &self,
        s: &mut Scratch,
        job: TileJob,
        roots: &[&Path],
        palettes: &[(&[u8], &[u8])],
    ) -> Result<()> {
        let tile = self.sampler.tile;
        let t = tile as u32;
        s.encoder
            .encode_tail(&s.classes, tile, tile, self.depth, &mut s.tail)?;
        for (root, (plte, trns)) in roots.iter().zip(palettes) {
            s.file.clear();
            s.file.extend_from_slice(&header_bytes(t, t, self.depth));
            append_palette_chunks(&mut s.file, plte, trns);
            s.file.extend_from_slice(&s.tail);
            let dir = root.join(job.z.to_string()).join(job.tx.to_string());
            fs::create_dir_all(&dir)?;
            fs::write(dir.join(format!("{}.png", job.ty)), &s.file)?;
        }
        Ok(())
    }
}
