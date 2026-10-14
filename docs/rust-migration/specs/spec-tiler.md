# spec-tiler: behavioral port spec for radar-ng tile rendering

Target: a Rust engineer implements the tile renderer from this document alone.
Source of truth: `/mnt/user-data/uploads/radar-ng` (read-only).

| File | Lines | Role |
|---|---|---|
| `backend/shared/tiler.py` | 2418 | geometry, sampling, classification, PNG emission, identity, atomic publish |
| `backend/shared/png_palette.py` | 79 | PLTE/tRNS rewrite, chunk iteration |
| `backend/shared/palettes.py` | 41 | palette loader, `PALETTES` env parsing |
| `backend/shared/palettes/{classic,muted,vivid}.json` | 135/133/133 | shipped palettes |
| `backend/shared/color_tables.json` | 67 | legacy palette, fallback for `classic` only |
| `backend/shared/test_tiler*.py`, `test_png_palette.py` | 94+174+111+1411+102 | 79 test items (section 17) |

**Status.** The five test modules pass against the source tree: **79 passed** in 4.2 s (system `/usr/bin/python3`, numpy 2.5.3, Pillow 12.3.0, SciPy 1.18.1, pyproj 3.8.0, pytest 9.1.1). The source tree was not modified. Items marked *(observed)* were checked by probing the reference implementation; everything else is read from code.

**Terms.** *Frame*: one source grid at one valid time. *Pyramid*: one palette's tree `{z}/{x}/{y}.png` for one frame. *Palette set*: the palettes rendered together for one frame. *Final path*: the caller-chosen pyramid root, published atomically.

**Out of scope.** Callers (`ingest_mrms`, `ingest_hrrr`, `ingest_airquality`, `nowcast`), `manifest.py`, storm code. Callers are referenced only for call patterns.

---

## 1. Conventions

1. **Tiling.** XYZ slippy map, Web Mercator. For zoom `z`, `n = 2**z`. Tile `(x, y)` covers longitudes `[x/n*360 - 180, (x+1)/n*360 - 180)`. Row `y = 0` is the north edge; `y` grows south. Default tile size `T = 256`.
2. **Angles** are degrees. `lats` and `lons` are cell-center coordinates. A source axis may be ascending or descending, and must be strictly monotonic.
3. **Source grid kinds.**
   - *Regular*: `lats` 1-D length `H`, `lons` 1-D length `W`, `source_crs` None.
   - *Projected*: `lats`, `lons` are 2-D `H×W` meshes of EPSG:4326 coordinates. `source_crs` is a CRS string. `source_x` (length `W`) and `source_y` (length `H`) are 1-D, in source-CRS units. Transform EPSG:4326 to the source CRS with `always_xy=True` (input order lon, lat).
   - `source_crs`, `source_x`, `source_y` must be passed together or not at all.
4. **Class index** is `uint8`. **Class 0 is transparent in every palette.** `idx.any()` means "has visible content".
5. **Missing data.** NaN and ±inf are missing for physical values. Categorical integers have no missing marker: out-of-range values map to class 0.
6. **Intervals** are half-open `[min, max)`.
7. **Numeric types matter.** `int` and `float` are kept distinct from the palette JSON, and this changes identity digests (section 10.1).

---

## 2. Public surface

Signatures are Python's. "Side effects" lists disk writes only. In-memory caches are in section 15.

| Name | Inputs (dtype, units, NaN) | Output | Side effects |
|---|---|---|---|
| `apply_color_table(data f32/f64 HxW, color_table)` | `{"ranges": [...], "no_data_below": x}` | u8 HxWx4 RGBA | none |
| `apply_categorical_color_table(data int HxW, categories, category_map)` | category ids | u8 HxWx4 RGBA | none |
| `ClassModel` (frozen): `edges` (f64 array, or None when categorical), `remap` (u8 array), `luts` (palette to u8 (K+1)x4) | | methods `plte(name)`, `trns(name)`, `classify(data)`, property `n_classes` | none |
| `build_class_model(color_tables: dict[palette, table])` | one `ranges` table per palette | `ClassModel` | none |
| `build_categorical_class_model(color_tables, category_map)` | `{name: {"categories": {...}}}` | `ClassModel` | none |
| `render_tiles(rgba u8 HxWx4, lats, lons, output_dir, zoom_levels, tile_size=256, resample=ignored, source_crs=None, source_x=None, source_y=None)` | legacy bilinear RGBA | tile count | **writes RGBA PNGs directly** (not atomic) |
| `render_tiles_atomic(rgba, lats, lons, output_dir, zoom_levels, *, source_id="direct-legacy-rgba-grid", publication_lock_root=None, **kwargs)` | as above | tile count (0 = nothing published) | atomic publish (13) |
| `render_indexed_tiles(idx u8 HxW, lats, lons, output_dirs: dict[palette, root], luts, zoom_levels, tile_size=256, source_crs, source_x, source_y)` | class index, nearest | tile count, same for every palette | **writes P-mode PNGs directly** (not atomic) |
| `render_continuous_tiles(data f32/f64 HxW, lats, lons, output_dirs, model, zoom_levels, tile_size=256, source_crs, source_x, source_y, nodata_value=None, min_valid_weight=1.0)` | physical values, bilinear, then classify | tile count | writes directly (not atomic) |
| `render_indexed_tiles_atomic(idx, lats, lons, output_dirs, luts, zoom_levels, *, source_id="direct-indexed-grid", publication_lock_root=None, **kwargs)` | | `MultiPaletteRenderResult` | atomic multi-palette publish |
| `render_continuous_tiles_atomic(data, lats, lons, output_dirs, model, zoom_levels, *, source_id="direct-continuous-grid", publication_lock_root=None, **kwargs)` | | `MultiPaletteRenderResult` | atomic multi-palette publish |
| `render_footprint_max_tiles(data f32 HxW, lats, lons, output_dirs, color_tables, zoom_levels, *, indexed: bool, nodata_value=None, tile_size=256)` | max-pooled overview, regular 1-D axes | tile count | writes directly (not atomic) |
| `render_frame_palettes(data, lats, lons, color_tables: dict[palette, table], output_dirs: dict[palette, root], zoom_levels, *, category_map=None, nodata_value=None, min_valid_weight=1.0, renderer="indexed", source_id="unspecified-frame", publication_lock_root=None, overview=None, **kwargs)` | **production entry point** | `MultiPaletteRenderResult` | atomic publish |
| `frame_pyramid_identity_is_compatible(identity, *, renderer, source_id, color_tables, palette_name, zoom_levels, tile_size=256, category_map=None, nodata_value=None, min_valid_weight=1.0, overview=None)` | resume check; no source grid | bool | none |
| `complete_pyramid_identity(path)` | | `PyramidIdentity`, or None when absent, unbound, or invalid | none |
| `is_complete_pyramid(path, *, renderer=None)` | | bool | none |
| `tile_renderer_for_role(role)` | role in `mrms`, `nowcast`, `hrrr`, `aux` | `"legacy"` or `"indexed"` (section 14) | none |
| `tile_geometry_cache_stats()`, `clear_tile_geometry_cache()` | | stats dict | process cache |
| `encode_indexed_png(tile u8 HxW, plte: bytes, trns: bytes)` | | P-mode PNG bytes | none |
| `PublishStatus` (str enum) | `created`, `existing_valid`, `existing_compatible`, `adopted`, `empty` | | |
| `PalettePublishOutcome(status, tile_count)`; `MultiPaletteRenderResult(outcomes)` with `.counts` (palette to count) and `.rendered_palettes` (count > 0, insertion order) | | | |
| `PyramidIdentity` (10 str fields, section 10.4): `to_document()`, `from_document()`, `is_renderer_compatible_with()` | | | |
| `PyramidInspection(tile_count, tile_paths: set[str], identity)` | | | |
| Exceptions `UnboundPyramidError(RuntimeError)`, `PyramidIdentityConflict(RuntimeError)` | | | |

`png_palette.py`: `PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"`; `iter_chunks(png)` yields `(type, payload)` and validates structure, not CRCs; `make_chunk(type, payload)`; `replace_palette(png, rgb, alpha|None)` (7.3).

`palettes.py`: `get_palette_names()`, `load_palette(name)` (6.1).

---

## 3. Tile geometry

### 3.1 Lat/lon to tile index (`_lat_lon_to_tile`)

```
n = 2**z
x = int((lon + 180.0) / 360.0 * n)
y = int((1.0 - ln(tan(lat_rad) + 1.0/cos(lat_rad)) / pi) / 2.0 * n)   # lat_rad = radians(lat)
x, y = clamp(x, 0, n-1), clamp(y, 0, n-1)
```

`int()` truncates toward zero; for in-range input this is floor. Clamping is the only range handling (no longitude wrap). `lat = ±90` does not raise.

### 3.2 Tile bounds (`_tile_bounds`)

```
west  = x/n*360 - 180            east  = (x+1)/n*360 - 180
north = degrees(atan(sinh(pi*(1 - 2*y/n))))
south = degrees(atan(sinh(pi*(1 - 2*(y+1)/n))))
```

Division by `n = 2^z` is exact in binary floating point, so the algebraic forms agree bit-for-bit.

### 3.3 Pixel-center grid

Pixel centers are `px_j = (j+0.5)/T`, `j = 0..T-1`.

```
lon_j = (tx + px_j)/n*360.0 - 180.0                       # evaluation order matters
lat_i = degrees(atan(sinh(pi * (1.0 - 2.0*(ty + px_i)/n))))
```

Row `i` runs north to south; column `j` runs west to east.

### 3.4 Source axis mapping (`_axis_to_fractional_indices(values, axis)`)

Maps coordinates onto a monotonic axis of length `N >= 2`, returning fractional indices. Out-of-range values extrapolate linearly (no clipping).

1. `steps = diff(axis)`. All `> 0`: `ordered = axis`, `pos = [0..N-1]`. All `< 0`: `ordered = axis[::-1]`, `pos = [N-1..0]`. Otherwise error "strictly monotonic".
2. **Affine fast path**, if `allclose(steps, steps[0], rtol=1e-7, atol=|steps[0]|*1e-10)`:
   `idx = ((v - axis[0]) / (axis[-1] - axis[0])) * (N-1)`. Divide, then multiply. The source calls this "byte-identical with the shipped renderer", so do not substitute a formula that rounds differently. Works for descending axes too (the span is negative).
3. **Piecewise path**: `seg = clamp(searchsorted(ordered, v, 'right') - 1, 0, N-2)`; `frac = (v - ordered[seg]) / (ordered[seg+1] - ordered[seg])`; `idx = pos[seg] + frac*(pos[seg+1] - pos[seg])`. NaN in gives NaN out.

### 3.5 Finite index bounds (`_finite_index_bounds(values, size)`)

Keep finite values. None if none remain. None if `max < 0` or `min > size-1`. Otherwise `start = clamp(floor(min), 0, size-1)`, `end = clamp(ceil(max), 0, size-1)`; None if `end <= start` (hazard H9). Return `(start, end)` inclusive.

### 3.6 Tile ranges per zoom

For each `z` in `zoom_levels` (given order):

```
(tx_min, ty_min) = tile(lat_max, lon_min, z)
(tx_max, ty_max) = tile(lat_min, lon_max, z)
for tx in tx_min..=tx_max (outer), ty in ty_min..=ty_max (inner)
```

Indexed and continuous paths take the bbox with `nanmin`/`nanmax` (infinities filtered only if needed). The legacy `render_tiles` uses `lats.min()`/`max()`, so a NaN coordinate raises `int(nan)` (H8). `zoom_levels` is not validated; Rust should reject `z` outside `0..=30`.

---

## 4. Sampling

### 4.1 Nearest (`_nearest_index(coord, n)`)

```
valid = (coord >= 0) & (coord <= n-1)           # closed interval, no half-pixel extension
out   = min(floor(where(valid, coord, 0) + 0.5) as i32, n-1)   # round half up
out[~valid] = -1
```

Regular grids store 1-D `rows`, `cols` (length `T`, `i32`). Projected grids store `flat = row*W + col` (`i32`, `-1` invalid), computed in `i64` then cast.

`sample_nearest`: regular: `tile[i][j] = idx[max(rows[i],0)][max(cols[j],0)]`, zeroed where `rows[i] < 0` or `cols[j] < 0`. Projected: `tile = idx.flat[max(flat,0)]`, zeroed where `flat < 0`. The source must be C-contiguous.

### 4.2 Bilinear physical sampling (`sample_bilinear`), exact arithmetic

Samples physical values; classes are never interpolated. Bit-exact parity requires this exact sequence.

Inputs: `values` cast to `f32` first (even from `f64`); coordinates `rows`, `cols` stored as **`f32`**. Output: `f32` `T×T`, NaN where not emitted.

Per pixel:

1. `coord_valid = 0 <= r <= H-1 and 0 <= c <= W-1` on the f32 coordinates.
2. `r_floor = floor(r) as i32`, `c_floor = floor(c) as i32`.
3. `rf = (f64)r - (f64)r_floor`; `cf = (f64)c - (f64)c_floor` (numpy promotes `f32 - i32` to f64).
4. `r0 = clamp(r_floor, 0, H-1)`, `r1 = clamp(r_floor+1, 0, H-1)`; likewise `c0`, `c1`.
5. Samples, in order: `s00 = v[r0][c0]`, `s01 = v[r0][c1]`, `s10 = v[r1][c0]`, `s11 = v[r1][c1]`.
6. Weights, same order, f64: `w00 = (1-rf)*(1-cf)`, `w01 = (1-rf)*cf`, `w10 = rf*(1-cf)`, `w11 = rf*cf`.
7. For each `(s, w)` in order, with `numerator` and `valid_weight` **stored as f32** after each step:
   ```
   valid = isfinite(s) && (nodata is None || s != f32(nodata))     # nodata test only if nodata is finite
   numerator    = f32( (f64)numerator    + (valid ? (f64)s : 0.0) * w )
   valid_weight = f32( (f64)valid_weight + (valid ? 1.0 : 0.0) * w )
   ```
8. `good = coord_valid && valid_weight >= f32(min_valid_weight - 1e-6)` (the subtraction is in f64, then cast).
9. `out = good ? f32(numerator / valid_weight) : NaN` (f32 division).

Semantics: with the default `min_valid_weight = 1.0`, a pixel is emitted only when all non-zero-weight support is valid, so NaN and fill values never bleed. An invalid corner with weight 0 contributes nothing (H10).

### 4.3 Legacy RGBA bilinear (`render_tiles`)

Per channel: `scipy.ndimage.map_coordinates(rgba[:,:,b], [rows, cols], order=1, mode="constant", cval=0)` on `uint8`. *(observed)* SciPy rounds `uint8` output to nearest with ties upward (0.5 gives 1, 1.5 gives 2, 76.5 gives 77). Non-premultiplied RGBA: transparent neighbors darken edges (legacy artifact). Skip a source region whose `max(alpha) == 0`, and skip a tile whose `max(alpha) == 0`.

### 4.4 Overview: max-pooled footprint (`render_footprint_max_tiles`)

Used only for regular 1-D continuous grids with `overview="max"`. It pools source cells that are finer than one output pixel.

**Zoom selection** (`_max_block_factors`):

```
px_deg  = 360.0 / (T * 2**z)
dlat    = |lats[1] - lats[0]|;  dlon = |lons[1] - lons[0]|
min_cos = cos(radians(min(89.0, max(|lats.min()|, |lats.max()|))))
block   = (max(1, int(px_deg*min_cos/dlat)), max(1, int(px_deg/dlon)))
```

A zoom is **pooled** iff `block != (1,1)`; otherwise **native** (sampled per 4.2). Test values: lats `20+0.01i` (3500), lons `-130+0.01i` (7000) give `(5,8)` at z4, `(2,4)` at z5, `(1,1)` at z7.

**Footprint edges** (`_footprint_edges(edge_coords, axis)`), `T+1` pixel edges:

```
frac  = _axis_to_fractional_indices(edge_coords, axis)
lo    = min(frac[k], frac[k+1]);  hi = max(frac[k], frac[k+1])      # pixel k
start = clamp(floor(lo + 0.5), 0, N)
stop  = clamp(floor(hi + 0.5), 0, N)
thin  = (stop <= start) & (hi >= -0.5) & (lo <= N - 0.5)
stop  = where(thin, min(start+1, N), stop)
start = where(thin & (start >= N), N-1, start)
```

Cells `[start, stop)` belong to the pixel. `stop <= start` after `thin` means off-grid. A pixel narrower than one cell still takes the cell under it.

**Pooling**:

1. `values` are `f32`; values equal to `f32(nodata)` become NaN.
2. Column edges `e_j = (tx + j/T)/n*360 - 180` (`j = 0..T`, edges not centers). For each pixel column, `(c_start, c_stop)` from `lons`. `cols[:, j] = nanmax(values[:, c_start_j : c_stop_j])`. Empty range gives NaN.
3. Row edges `lat_edges_i = degrees(atan(sinh(pi*(1 - 2*(ty+i/T)/n))))`. For each pixel row, `(r_start, r_stop)` from `lats`. `tile[i][j] = nanmax(cols[r_start_i : r_stop_i, j])`.
4. Classify `tile` (5.3, f32). Skip if `!tile.any()`.

The reduction is `fmax` (NaN-ignoring; all-NaN gives NaN). The max is over physical values before classification: "strongest value in the footprint".

### 4.5 Overview validation (`render_frame_palettes`)

An `overview` value other than `"max"` raises `ValueError`, first in `_frame_render_contract`. With `overview` set, `ValueError` is also raised if the grid is categorical, `source_crs` is set, or `lats`/`lons` is not 1-D. An unknown renderer raises `ValueError("unknown tile renderer ...")` from the same function.

---

## 5. Classification model

### 5.1 `build_class_model(color_tables)`

Input: `{palette: {"ranges": [{"min","max","rgba"}...], "no_data_below": x}}`. `no_data_below` defaults to `-999` when absent.

1. **Edges**: sorted unique set of all range `min`, all range `max`, and all `no_data_below`, as f64. `N_e = len(edges)`.
2. **Raw index** `k` in `0..N_e`: `k=0` means `v < edges[0]`; `1 <= k <= N_e-1` means `edges[k-1] <= v < edges[k]`; `k = N_e` means `v >= edges[N_e-1]`. `raw_n = N_e + 1`.
3. **Per-palette LUT**: for `i in 1..N_e-1`, with `lo = edges[i-1]`, `hi = edges[i]`: skip if `lo < no_data_below`. Otherwise, for each range in declaration order where `range.min <= lo and hi <= range.max`, set `lut[i] = rgba`. **Later ranges win** on overlap. Raw `0` and `N_e` stay transparent.
4. **Collapse** (`_collapse_transparent`): `visible[k] = any palette has alpha > 0 at k`. Keep visible raw indices in ascending order, renumber `1..K`. Class 0 is `[0,0,0,0]` everywhere. If `K > 255`, raise `ValueError("... exceed the uint8 palette budget")`.
5. **Output**: `remap` (u8, length `raw_n`, raw to class) and `luts[palette]` (u8 `(K+1)x4`, row 0 transparent). `n_classes = K+1`.

A visible class can have `alpha = 0` in some palette. It is then transparent there, but nonzero in the shared index (section 8).

### 5.2 `build_categorical_class_model(color_tables, category_map)`

`raw_n = max(v for v in category_map if v >= 0, default 0) + 1`. Per palette: `lut[v] = categories[name]` for each `(v, name)` where `v >= 0` and `name in categories` (unknown names silently dropped). Collapse as above. `edges = None`. `classify` maps `v < 0` or `v > raw_n-1` to 0.

### 5.3 `ClassModel.classify(data) -> u8`, same shape

Chunked to bound temporaries: 2-D inputs in row blocks of `max(1, (1<<21) // ncols)`; others in flat blocks of `1<<21` cells.

- **Categorical** (`edges is None`): `v = int64(src)`; `v = 0` where `v < 0 or v > top`; `out = remap[v]`. NaN has no defined category; treat as 0 in Rust.
- **Continuous**: `idx = searchsorted(edges, v, 'right')` (count of edges `<= v`); `out = remap[idx]`.
  - **Precision rule**: if the input is float and not `f64`, cast the edges to the input dtype (f32 for f32 input) **only if** the cast edges are still strictly increasing. Otherwise use f64 edges.
  - NaN and ±inf give class 0 (`+inf` gives `idx = N_e`, transparent; `-inf` gives 0).

### 5.4 LUT byte exports

`plte(name) = luts[name][:, 0:3]` as bytes (3 per entry). `trns(name) = luts[name][:, 3]` as bytes (1 per entry). All palettes in one model have the same row count.

---

## 6. Palette data

### 6.1 Loader (`palettes.py`)

- `get_palette_names()`: env `PALETTES` (default `"classic"`), split on `,`, strip, drop empties; if nothing remains return `["classic"]`. Duplicates are kept.
- `load_palette(name)`: (1) if `backend/shared/palettes/{name}.json` exists, parse and return the whole object; (2) else, if `name == "classic"` and `color_tables.json` exists, parse and return it; (3) else `FileNotFoundError("palette not found: {name} (looked in {path})")`.
- The name is joined into a path without validation (H13).
- Callers take one color key from the object (`["reflectivity"]` for MRMS, `["pm25"]` for air quality). The per-key dict is what the renderer receives and what is digested.

### 6.2 Palette JSON schema

Top level: object. Keys starting with `_` are ignored (top-level `_comment` is in no digest). Every other key is a **color key** whose value is an object:

- `"unit"`: string, informational. **In the palette digest** (10.2).
- `"ranges"`: array of `{"min": number, "max": number, "rgba": [r,g,b,a]}`. `rgba` entries are integers `0..255`. Out-of-range values raise `OverflowError` at LUT build time (verified for 256 and -1). Float entries truncate toward zero (verified: 127.9 gives 127).
- `"no_data_below"`: number. Defaults to `-999` in the model when absent. In the digest only if present.
- `"categories"`: alternative to `ranges`. Object `name -> [r,g,b,a]`.
- `"_comment"` inside a color key: ignored by the model, **but in the digest** (H2).

Python `json` keeps `5` as `int` and `5.0`, `0.01` as `float`. The digest sees the difference (10.1).

### 6.3 Shipped inventory

All three palettes share edges per color key (enforced by `test_all_shipped_palettes_share_bin_edges_per_color_key`). A new palette must match.

| Color key | Unit | Ranges | no_data_below | Notes |
|---|---|---|---|---|
| `reflectivity` | dBZ | 13: 5-10 through 60-65 in steps of 5, then 65-75 (edges 5,10,...,65,75) | 5 | NWS-aligned bands |
| `temperature` | fahrenheit | 11: -40..130 | -999 | |
| `wind_speed` | mph | 8: 0..150 | 0 | |
| `cape` | J/kg | 5: 500..8000 | 500 | |
| `precip_type` | none | `categories`: rain, snow, freezing_rain, ice_pellets, hail | n/a | |
| `precip_accum` | in | 8: 0.01..20 | 0.01 | absent from legacy file |
| `cloud_cover` | % | 5: 10..101 | 10 | |
| `pm25` | µg/m³ | 12: 0..20000 (EPA breakpoints) | 0 | classic has `_comment`s inside the key |
| `ozone` | ppb | 11: 0..600 | 0 | classic has a `_comment` inside the key |

`classic.json` has a top-level `_comment`; `muted` and `vivid` have one too. The legacy `color_tables.json` (fallback for `classic` only) has `reflectivity` with `no_data_below: 15` (classic uses 5) and different colors, plus `temperature`, `wind_speed`, `cape`, `precip_type`. It lacks `precip_accum`, `cloud_cover`, `pm25`, `ozone`. Falling back to it changes output.

### 6.4 Interval semantics

Half-open `[min, max)`: `min` belongs to its range, `max` does not (reflectivity 75 is transparent). Values below `no_data_below` are transparent even if a range covers them. Gaps are transparent. Overlaps: the later range wins. No color interpolation.

---

## 7. PNG

### 7.1 Indexed encoding (`encode_indexed_png(tile, plte, trns)`)

Encoder: Pillow `Image.frombuffer("P", (W,H), bytes, "raw", "P", 0, 1)`, `putpalette(plte)`, `save(..., "PNG", transparency=trns, optimize=False, compress_level=PNG_COMPRESS_LEVEL)`.

- **Bit depth** (observed for N = 1, 2, 3, 5, 12, 16, 17, 200, 256; boundary 4 inferred), with `N = K+1` palette entries: `N <= 2` gives 1; `N <= 4` gives 2; `N <= 16` gives 4; `N > 16` gives 8. Pixels are packed to that depth. `K` is shared across palettes, so one tile gives the same depth for every palette.
- **Chunks** *(observed)*: `IHDR, PLTE, tRNS, IDAT, IEND`. For `N = 256` the IDAT stream was split into two chunks; smaller palettes gave one.
- **PLTE** length `3*N` bytes (not padded to 256). **tRNS** length `N` bytes (observed; the all-255 case was not tested).
- **Compression**: zlib level `PNG_COMPRESS_LEVEL` (env, default 6).
- **Filters** *(observed)*: per-row filter bytes are chosen by Pillow. For bit depths 1, 2, 4, types `{0,1,2,4}` appeared (4 = Paeth). For bit depth 8 on random data only `0` appeared. Any valid filter is fine for pixel content (see H6 for the exception).
- No other ancillary chunks.

### 7.2 Legacy RGBA PNG

`Image.fromarray(rgba, "RGBA")`, `save(..., "PNG", optimize=False, compress_level=1)`. Mode 6.

### 7.3 Palette rewrite (`replace_palette(png, rgb, alpha)`)

Purpose: derive another palette's PNG from an encoded indexed PNG without re-encoding IDAT.

Validation: `len(rgb) % 3 == 0` and `1 <= len(rgb)/3 <= 256`; `alpha` (if given) has at most as many bytes as palette entries. Otherwise `ValueError`.

Algorithm:

1. Write signature. Walk chunks with `iter_chunks`.
2. `IHDR`: if colour type `!= 3`, raise `"PNG colour type X is not indexed (3)"`. Else copy with fresh CRC.
3. `PLTE`: emit new `PLTE(rgb)`; if `alpha` is given, emit `tRNS(alpha)` immediately after.
4. Original `tRNS`: raise `"tRNS before PLTE"` if seen before `PLTE`; otherwise drop it.
5. `IDAT` before `PLTE`: raise `"indexed PNG has no PLTE chunk"`. Any other chunk: copy with fresh CRC.
6. At end, no `PLTE` seen: raise `"indexed PNG has no PLTE chunk"`.

Guarantees (tested): IDAT and IHDR payloads byte-identical to the source; every CRC equals `crc32(type||payload)`; `alpha=None` removes tRNS; decoded pixels equal a direct encode with the new palette.

### 7.4 Chunk format

`length (u32 BE) | type (4) | data | crc32(type||data) (u32 BE)`. `iter_chunks` errors: `"not a PNG: bad signature"`, `"truncated PNG chunk header"`, `"truncated PNG chunk {type!r}"`.

### 7.5 Sidecars

**No `.gz` or other precompressed sidecars are written by any tiler code.** `backend/api/Caddyfile` has `precompressed gzip`, which serves an existing `<file>.gz` if present. Nothing creates one. Do not add them: PNG is already deflated.

---

## 8. Tile emission and elision

| Path | Before sampling | After sampling | Emitted when |
|---|---|---|---|
| `render_tiles` (legacy) | regular: all cols or rows out of range, skip; source region `alpha.max()==0`, skip | tile `alpha.max()==0`, skip | some alpha > 0 |
| `render_indexed_tiles` | `_region_transparent(idx[r0:r1+1, c0:c1+1])`, skip | `!tile.any()`, skip | some class > 0 |
| `render_continuous_tiles` | none | `!tile.any()`, skip | some class > 0 |
| `render_footprint_max_tiles` | no column/row with a non-empty range, skip | `!tile.any()`, skip | some class > 0 |

`_region_transparent(region)`: empty is transparent; if `size > 1<<22` and `region[::8, ::8].any()`, return False (strided probe); else `!region.any()`.

Consequences:

- A tile is written **for every palette** if its class index is nonzero, even where that class is transparent in some palettes. This is intended.
- Indexed and continuous counts are equal across palettes. Legacy counts can differ, because alpha is tested per palette.
- Only fully transparent tiles are skipped. Partly transparent tiles are emitted.

---

## 9. Render algorithms (non-atomic)

### 9.1 Tile iteration (`_iter_tile_geometry`)

Yields `(z, tx, ty, geometry)` in order: `z` (given order), `tx` ascending, `ty` ascending. Geometry comes from the process cache (15.1). `None` geometries are skipped. The transformer is created lazily, once per call.

`_compute_tile_geometry`:

- **Regular, bilinear**: `cols_1d`, `rows_1d` from 3.4. Reject if all `cols < 0` or all `cols > W-1` (same for rows). Bounds from 3.5. Store `rows`, `cols` as `f32` (length `T`).
- **Regular, nearest**: `rows = _nearest_index(rows_1d, H)`, `cols = _nearest_index(cols_1d, W)` (`i32`). Reject if no valid row or no valid column.
- **Projected**: bbox-reject the tile against the grid's finite lat/lon bbox (`east < lon_min or west > lon_max or north < lat_min or south > lat_max` gives None). Transform the tile's pixel-center mesh to source CRS (`xs, ys`). Map with 3.4 against `source_x`, `source_y`. Non-finite results become `-1.0`. Bounds from 3.5.
  - Bilinear: `rows`, `cols` as `f32` `T×T`.
  - Nearest: `flat = row*W + col` as `i32`, `-1` where invalid. Reject if no valid pixel.

### 9.2 `render_indexed_tiles`

1. Empty `output_dirs`: return 0. `idx` must be 2-D; make it C-contiguous `u8`.
2. Palette order is `list(output_dirs)`. Missing LUT keys raise `ValueError("missing lookup tables for palettes: [...]")`.
3. For each tile (9.1, nearest): precheck region; sample; skip if `!tile.any()`; write.
4. `_write_index_tile`: encode the **first** palette's PNG once. Every other palette's body is `replace_palette(png, plte[p], trns[p])`. Write to `{root_p}/{z}/{tx}/{ty}.png`. `made_dirs` caches `mkdir` per call.
5. Return the emitted tile count (same for every palette).

### 9.3 `render_continuous_tiles`

As 9.2, except: `data` must be 2-D; `model.edges` must not be None (else `ValueError("continuous rendering requires a range class model")`); tiles use bilinear geometry; each tile is sampled per 4.2, then `model.classify`, then skipped if `!tile.any()`.

### 9.4 `render_footprint_max_tiles`

Values to `f32`, nodata to NaN. The model is built from `color_tables[p]` for `p` in `output_dirs` only (not the caller's full set; H3). Per 4.4. `indexed=True` writes P-mode via `_write_index_tile`. `indexed=False` writes RGBA per palette via `luts[p][tile]`, compress level 1.

### 9.5 `render_frame_palettes` decision tree

1. Empty `output_dirs`: return an empty result. No identities.
2. `tables = {p: color_tables[p] for p in output_dirs}` (`KeyError` if missing).
3. `_frame_render_contract` (10.2): validates `overview`, builds the model for `indexed`, returns `(algorithm, semantic_policy, policy, model)`.
4. With `overview`: validate (4.5); split zooms into `pooled_zooms` (4.4) and `native_zooms`.
5. Compute identities (`_publication_identities`). This hashes the full source array (15.4).
6. Branch:
   - `legacy`: per staged palette, pooled count plus `render_tiles` on colorized RGBA (`apply_color_table` or `apply_categorical_color_table`). Pooled tiles are written once for all palettes.
   - `indexed` with `category_map`: `idx = model.classify(data)`, then `render_indexed_tiles(idx, ..., zoom_levels)`. Overview is not allowed here.
   - `indexed` continuous: `_render_pooled` (if any pooled zooms) plus `render_continuous_tiles(..., native_zooms)`.
7. All of the above runs inside `_render_and_publish_atomic` (13).

---

## 10. Identity

### 10.1 Canonical JSON and digests

`_stable_digest(v) = sha256_hex(json.dumps(_stable_json_value(v), separators=(",",":"), sort_keys=True, ensure_ascii=True).encode())`.

`_stable_json_value`, applied in order:

1. `np.ndarray` becomes `{"array_digest": <hex>, "dtype": dtype.str, "shape": [...]}`.
2. `np.generic` becomes `.item()` and recurses. (Checked before `float`, since `np.float64` subclasses `float`.)
3. `bytes` becomes `{"bytes": hex}`.
4. `Mapping` becomes an object with `str(key)` keys.
5. `list`/`tuple` becomes an array.
6. Non-finite float becomes `{"float": repr(x)}`: `{"float":"nan"}`, `"inf"`, `"-inf"`.
7. `None`, `str`, `int`, `bool`, finite `float` pass through.
8. Anything else becomes `str(value)`.

**Float format.** Finite floats use Python `repr`: shortest round-trip digits; exponent form when the decimal exponent is below -4 or at least 16, written as `e`, sign, and at least two exponent digits (`1e+16`, `1e-05`); otherwise fixed with at least one fractional digit (`1.0`, `0.01`). **`int 1` and `float 1.0` hash differently** (verified). `serde_json` and `ryu` format differently, so Rust needs its own formatter.

**ASCII.** `ensure_ascii=True`: non-ASCII becomes `\uXXXX` (verified: `µ` becomes `µ`).

**Array digest** (`_array_content_digest`): `sha256` over `json.dumps({"dtype": dtype.str, "shape": list(shape)}, separators=(",",":"), sort_keys=True)` followed by the raw C-order bytes (row chunks for non-contiguous input). `dtype.str` examples: `|u1`, `<i2`, `<f4`, `<f8`. `f32` and `f64` copies of the same values hash differently (verified).

**Coordinate fingerprint** (`_array_fingerprint`): `(shape, dtype.str, blake2b(bytes, digest_size=16))`. Contiguous input.

**Grid digest** (`_grid_spec_digest`):

```
_stable_digest({
  "kind": "regular" | "projected", "height": H, "width": W,
  "lat_bounds": [min, max], "lon_bounds": [min, max],          # finite bbox, floats
  "coordinate_identity":
     regular:   [fingerprint(lats), fingerprint(lons)]
     projected: [str(source_crs), fingerprint(source_x), fingerprint(source_y)]
})
```

The sampling mode is deliberately excluded, so a rollback can recognize a foreign winner.

**Tile-spec digest**: `_stable_digest({"tile_size": T, "zoom_levels": [z...]})`. Zoom order matters.

**Per-palette identity** (`_publication_identities`): `PyramidIdentity(renderer, algorithm, source_id, source_digest=_array_content_digest(source_data), grid_spec_digest, palette_name=p, palette_digest=_stable_digest(payload[p]), tile_spec_digest, semantic_digest=_stable_digest(semantic_policy), policy_digest=_stable_digest(policy))`. `source_id` must be non-empty.

### 10.2 Identity per entry point

Entry points that look alike do not always share identities (H1).

| Entry point | renderer | algorithm | default source_id | `semantic_policy` | `policy` | palette payload |
|---|---|---|---|---|---|---|
| frame, legacy, continuous | `legacy` | `rgba-bilinear-v1` | caller | `{kind: continuous, nodata_value, min_valid_weight}` | `{sampling: bilinear-rgba, category_map: null, png_compress_level: 1}` | per-key dict |
| frame, legacy, categorical | `legacy` | `rgba-bilinear-v1` | caller | `{kind: categorical, category_map}` | `{sampling: bilinear-rgba, category_map, png_compress_level: 1}` | per-key dict |
| frame, indexed, categorical | `indexed` | `category-nearest-v1` | caller | `{kind: categorical, category_map}` | `{sampling: nearest, category_map, model_remap, png_compress_level: L}` | per-key dict |
| frame, indexed, continuous | `indexed` | `physical-bilinear-classify-v1` | caller | `{kind: continuous, nodata_value, min_valid_weight}` | `{sampling: bilinear, nodata_value, min_valid_weight, model_edges, model_remap, png_compress_level: L}` | per-key dict |
| `render_tiles_atomic` | `legacy` | `rgba-bilinear-v1` | `direct-legacy-rgba-grid` | `{kind: precolorized-rgba}` | `{sampling: bilinear-rgba, png_compress_level: 1}` | `{"precolorized": true}`, palette name `rgba` |
| `render_indexed_tiles_atomic` | `indexed` | `class-index-nearest-v1` | `direct-indexed-grid` | `{kind: preclassified-categorical-index}` | `{sampling: nearest, png_compress_level: L}` | **LUT ndarray** |
| `render_continuous_tiles_atomic` | `indexed` | `physical-bilinear-classify-v1` | `direct-continuous-grid` | `{kind: continuous, nodata_value, min_valid_weight}` | `{sampling: bilinear, nodata_value, min_valid_weight, model_edges, model_remap, png_compress_level: L}` | **LUT ndarray** |

Notes: `L` is `PNG_COMPRESS_LEVEL` (H15). Frame-path `overview` adds `"overview": "max"` to both semantic and policy only when set, so existing identities are unchanged. Tile-spec uses `kwargs.get("tile_size", 256)`. `source_digest` hashes the full array: f32 physical for continuous, u8 class index for preclassified, int for categorical, u8 H×W×4 for legacy atomic.

### 10.3 Cross-renderer compatibility

`is_renderer_compatible_with(expected)` is true iff:

1. `self.renderer != expected.renderer`;
2. `{(self.renderer, self.algorithm), (expected.renderer, expected.algorithm))}` is exactly `{(legacy, rgba-bilinear-v1), (indexed, physical-bilinear-classify-v1)}` or `{(legacy, rgba-bilinear-v1), (indexed, category-nearest-v1)}`;
3. these fields are equal: `source_id`, `source_digest`, `grid_spec_digest`, `palette_name`, `palette_digest`, `tile_spec_digest`, `semantic_digest`.

`algorithm` and `policy_digest` are not compared. This lets a legacy rollback reuse indexed winners (`test_rollback_reuses_full_indexed_winner_without_claiming_legacy`).

`frame_pyramid_identity_is_compatible(stored, ...)` rebuilds an expected identity from the stored `source_digest` and `grid_spec_digest` (it has no grid), using the current contract, and returns `stored == expected or stored.is_renderer_compatible_with(expected)`.

### 10.4 `PyramidIdentity` document

`to_document()`: `{name: str(value)}` for the 10 fields, sorted by name. `from_document(doc)` requires exactly those 10 keys, all non-empty strings. Fields: `renderer`, `algorithm`, `source_id`, `source_digest`, `grid_spec_digest`, `palette_name`, `palette_digest`, `tile_spec_digest`, `semantic_digest`, `policy_digest`.

---

## 11. Completion marker

File: `{pyramid}/.render-complete-v2.json`. `_PYRAMID_COMPLETE_FILE = ".render-complete-v2.json"`, `_PYRAMID_MARKER_SCHEMA = 2`. `.render-complete.json` (schema 1) is read for diagnostics only and never written.

Body: one line, ASCII, `json.dumps(..., separators=(",",":"), sort_keys=True) + "\n"`:

```
{"identity":{...10 fields, sorted...},"schema_version":2,"tile_content_digest":"<hex>","tile_count":N,"tile_path_digest":"<hex>"}
```

- `tile_count`: number of `*.png` under the pyramid (`rglob`, any depth).
- `tile_path_digest` = `sha256_hex("\0".join(sorted(rel_paths)).encode())`, with `rel_paths` as `"{z}/{x}/{y}.png"`.
- `tile_content_digest` = `sha256` over each sorted rel path: `u32_BE(len(utf8 path)) || utf8 path || u64_BE(file size) || file bytes` (read in 1 MiB chunks).
- `identity` = the palette's `to_document()`.

**Write** (`_write_completion_marker`):

1. Create `{pyramid}/..render-complete-v2.json.tmp-{uuid4 hex32}` with exclusive create (note the double dot).
2. Write, flush, `fsync` the file.
3. `os.link(temp, marker)`: create-only. On `FileExistsError`, compare bytes; equal is fine, different raises `PyramidIdentityConflict("completion marker already binds a different identity: ...")`.
4. `fsync` the pyramid directory.
5. `finally`: unlink temp (ignore missing), `fsync` the directory.

**Read** (`_inspect_complete_pyramid(path, expected=None)`):

1. Not a directory: `RuntimeError`.
2. No `*.png`: `RuntimeError("published pyramid has no PNG tiles")`.
3. Marker missing: `UnboundPyramidError` (detail "legacy completion marker" if the v1 file exists, else "no identity-bound completion marker").
4. Parse failures (`OSError`, `KeyError`, `TypeError`, `ValueError`, `JSONDecodeError`): `RuntimeError("invalid pyramid completion marker")`.
5. `schema_version` must be int `2`: else `RuntimeError("unsupported pyramid completion schema N")`.
6. `tile_count` must equal the file count: else `RuntimeError("... count mismatch ...")`.
7. `tile_path_digest` must match: else `RuntimeError("pyramid completion path mismatch")`.
8. `identity` parsed via `from_document`.
9. `tile_content_digest` must match (reads every tile): else `RuntimeError("pyramid completion content mismatch")`.
10. `expected` given and `identity != expected`: `PyramidIdentityConflict("pyramid identity mismatch at ...")`.
11. Return `PyramidInspection(count, paths, identity)`.

Only `UnboundPyramidError` permits adoption. Every other failure is fail-closed.

---

## 12. On-disk layout

Caller-chosen roots. The MRMS call site uses `tile_base/layer/palette/timestamp` with lock root `tile_base/layer`.

| Artifact | Template | Created by | Notes |
|---|---|---|---|
| Final pyramid | `{output_dir}` (e.g. `{tile_base}/{layer}/{palette}/{timestamp}`) | atomic publish | immutable once published |
| Tile | `{output_dir}/{z}/{tx}/{ty}.png` | every writer | decimal, no zero padding |
| Staging | `{final.parent}/.{final.name}.tmp-{uuid4 hex32}` | atomic publish | hidden; `final.name` may contain `:` |
| Lock dir | `{lock_root}/.render-publish-locks/` | `_durable_mkdir` | default `lock_root = final.parent.parent` |
| Lock file | `{lock_root}/.render-publish-locks/{stripe:02x}.lock` | `flock` | stripe `0..63` (13.1) |
| Marker | `{final}/.render-complete-v2.json` | `_write_completion_marker` | written into staging before rename |
| Marker temp | `{final}/..render-complete-v2.json.tmp-{hex32}` | `_write_completion_marker` | removed in `finally` |

`ARCHITECTURE.md` describes staging as `<timestamp>.tmp-<uuid>`. The code uses `.<timestamp>.tmp-<hex>`; the code is authoritative. `tile_cleanup` (out of scope) matches `".tmp-" in name` and removes such directories older than 3600 s by mtime.

The lock directory is shared with Python workers. Rust must use the same path and stripe function (13.1).

---

## 13. Atomic publish protocol (`_render_and_publish_atomic`)

### 13.1 Locks (`_publication_group_lock`)

1. Canonicalize finals with `resolve(strict=False)`. Reject equal or nested finals: `ValueError("palette outputs must be distinct, non-nested paths: ...")`.
2. Lock root per palette: `lock_root.resolve(strict=False)` if given, else `final.parent.parent`. Each final must be a strict descendant (`root in final.parents`), else `ValueError("publication lock root must contain every output: ...")`.
3. Create `{root}/.render-publish-locks` with `_durable_mkdir` (13.8).
4. Stripe: `int.from_bytes(sha256(str(final).encode())[:2], "big") % 64`; file `{stripe:02x}.lock`.
5. Deduplicate and sort lock paths as strings. Open each with `"a+b"`, `flock(LOCK_EX)` in that order. Release in reverse.

The lock is held for the **entire render**, not only the publish (H17).

### 13.2 Classify existing finals

- Absent: unresolved.
- Present: `_inspect_complete_pyramid(F)`. `UnboundPyramidError`: unresolved (adoption candidate). Any other `RuntimeError`: raise (fail closed). Valid: `_fsync_bound_publication(F, root)`, mark bound.

| State | Action |
|---|---|
| All bound, all identities equal expected | return `existing_valid` for each. **No render.** |
| All bound, all compatible foreign (10.3), one shared render spec | return `existing_compatible` for each. **No render.** |
| All bound, otherwise | raise `PyramidIdentityConflict("complete palette set has mixed or incompatible render identities")` |
| Some bound, each bound identity exactly equal to expected | bound are `existing_valid`; render the rest |
| Some bound, any bound identity not exactly equal | raise `PyramidIdentityConflict("partial palette set contains an incompatible pyramid: ...")` |
| None bound | render all |

Foreign winners are reused only for a complete foreign set. Mixed or partial foreign sets fail closed (`test_partial_foreign_renderer_set_fails_closed`).

### 13.3 Stage and render

Staging for each unresolved `F`: `F.parent / f".{F.name}.tmp-{uuid4 hex32}"`. Call `render({name: staging})` once for all unresolved palettes. It returns an int (applied to all) or a mapping with exactly the staging keys (else `RuntimeError("renderer did not report every staged palette")`). Negative counts raise `RuntimeError`.

### 13.4 Validate staged trees

For each staged palette with count `c`:

1. `rglob("*.png")` on staging; if `len != c`: `RuntimeError("staged pyramid tile count mismatch ...")`.
2. `c == 0`: if `F` exists, `RuntimeError("existing unbound pyramid conflicts with an empty render: ...")`; otherwise mark `empty`.
3. Else `_fsync_pyramid`: fsync each tile, then each directory from deepest up, including the root.
4. `_write_completion_marker(staging, paths, identity)`.
5. `_inspect_complete_pyramid(staging, identity)` must succeed.

### 13.5 Decide every action before mutating

| Condition | Action |
|---|---|
| `c == 0` | `empty` |
| `F` absent | `create` |
| `F` unbound, same tile path set and same content digest as staging | `adopt` |
| `F` unbound and different | raise `PyramidIdentityConflict("pre-marker pyramid differs from the deterministic staging render and cannot be adopted: ...")` |
| `F` bound (appeared during render), identity equal | `existing` |
| `F` bound, identity different | raise `PyramidIdentityConflict("concurrent pyramid has incompatible identity: ...")` |

All palettes are decided first, so a late failure leaves nothing half-adopted.

### 13.6 Execute (in `unresolved` order)

- **empty**: remove staging (ignore errors). Outcome `empty`, count 0.
- **adopt**: `_fsync_pyramid(F)`, `_write_completion_marker(F, ...)` (into the existing final, in place), inspect with expected identity, `_fsync_bound_publication(F, root)`, remove staging. Outcome `adopted`.
- **existing**: remove staging. Outcome `existing_valid`.
- **create**: `os.rename(staging, F)`.
  - Success: `_fsync_directory_chain(F.parent, root)`, inspect with expected identity. Outcome `created`.
  - `OSError` with `errno` in `{EEXIST, ENOTEMPTY}` (another writer won): inspect `F`. Unbound: must match staging, else `PyramidIdentityConflict("concurrent pre-marker pyramid differs: ...")`; if it matches, write marker, inspect, `_fsync_bound_publication`, outcome `adopted`. Bound with exact identity: outcome `existing_valid`. Staging is removed in every sub-case.
  - Any other `OSError`: propagates.

Outcomes are returned in `output_dirs` order.

### 13.7 Failure handling

Any `BaseException` in the `try`: remove every staging directory (ignore errors), then re-raise. Palettes already renamed (`created` or `existing`) are **not** rolled back; they are complete and immutable. (`test_atomic_partial_publish_converges_on_retry`: classic renamed, muted fails, call raises, classic stays. Retry: classic `existing_valid`, muted `created`.) The lock is released in the enclosing `finally`.

### 13.8 Durability

- File: `fsync(fd)` on tile files and the marker. Errors are never suppressed.
- Directory `_fsync_directory(p)`: `open(p, O_RDONLY|O_DIRECTORY)`, `fsync`, close. `EINVAL` or `ENOTSUP` on either step is ignored; other errors raise.
- `_fsync_directory_chain(p, ancestor)`: fsync `p`, then each parent up to and including `ancestor`, stopping at the filesystem root.
- `_durable_mkdir(p)`: `mkdir(parents=True, exist_ok=True)` for the first missing ancestor onward, then `_fsync_directory_chain(p, ancestor)`.
- `_fsync_bound_publication(F, root)`: fsync `F`, then `_fsync_directory_chain(F.parent, root)`. Runs for every bound palette on every call, so an earlier failed fsync is repaired on retry.

Required order (tested): tile files synced, marker written, rename, parent-chain fsync after rename.

---

## 14. Environment and configuration

Module constants are read at import unless noted.

| Name | Default | Range | Effect |
|---|---|---|---|
| `TILE_GEOMETRY_CACHE_MB` | 256 | [0, 4096] | byte budget of the geometry cache (array bytes) |
| `TILE_GEOMETRY_CACHE_ENTRIES` | 4096 | [1, 1000000] | entry budget |
| `TILE_PNG_COMPRESS_LEVEL` | 6 | [0, 9] | indexed PNG zlib level; **part of indexed identity** |
| `PALETTES` | `classic` | any names | read at call time by `palettes.py` |
| `TILE_RENDERER` | `legacy` | `legacy` or `indexed` (case-insensitive, stripped) | read at call time by `tile_renderer_for_role` |
| `TILE_RENDERER_CANARY_ROLE` | empty | `mrms`, `nowcast`, `hrrr`, `aux` | required when `TILE_RENDERER=indexed` |

`_bounded_int_env(name, default, lo, hi)`: parse int; on `ValueError` (including empty) use default; clamp to `[lo, hi]`.

`tile_renderer_for_role(role)`:

1. `role` not in the four roles: `ValueError("unknown tile renderer role ...")`.
2. `TILE_RENDERER` not in `{legacy, indexed}`: `ValueError`.
3. `legacy`: return `legacy` (canary ignored).
4. `indexed`: canary must be one of the four roles, else `ValueError("TILE_RENDERER=indexed requires TILE_RENDERER_CANARY_ROLE=<...>")`. Return `indexed` iff `role == canary`, else `legacy`.

The canary rule is deliberate: `TILE_RENDERER=indexed` alone cannot enable the new renderer for every role at once.

---

## 15. Caching, concurrency, performance

### 15.1 Geometry cache (process-global)

- **Key**: `(GridSpec, tile_size, z, tx, ty)`. `GridSpec = (kind, height, width, lat_bounds, lon_bounds, coordinate_identity, sampling)`, frozen. Regular: fingerprints of full `lats` and `lons`. Projected: `(str(source_crs), fingerprint(source_x), fingerprint(source_y))`; the lat/lon meshes contribute only their finite bbox.
- **Value**: `_TileGeometry | None`. **`None` is cached** (negative caching).
- **LRU**: a hit moves the entry to the end. Insert is a no-op if the key exists. Evict oldest while `bytes > MB*2^20` or `entries > ENTRIES`, never evicting the last entry.
- **Byte sizes** (sum of `rows`, `cols`, `flat` nbytes): regular bilinear 2 KiB (`2*256*f32`); regular nearest 2 KiB (`2*256*i32`); projected bilinear 512 KiB (`2*256*256*f32`); projected nearest 256 KiB (`256*256*i32`).
- **Single-flight**: a miss registers an inflight `Event`; concurrent requests wait (`waits` counter). On success the value is cached and the event set. On exception the inflight entry is popped and waiters retry; a failing producer is not cached.
- **Stats**: `hits`, `misses`, `waits`, `evictions`, `entries`, `bytes`. `clear_tile_geometry_cache()` resets them.

Tested behavior: an identical second frame hits every key and calls no pyproj; a changed interior longitude with the same endpoints is a miss; a subsampled grid is a miss.

### 15.2 Lock striping

64 lock files (13.1), shared by unrelated frames under the same root.

### 15.3 Threading

Each Python call is single-threaded. Parallelism is at the caller: `NOWCAST_RENDER_WORKERS` threads each render a lead-time pyramid. PNG encoding and NumPy release the GIL. In Rust, tile-level parallelism within one frame is safe: each tile is its own file, and the emitted set (not order) is what matters. Keep `made_dirs` per call or make it thread-safe.

### 15.4 Performance facts

- Palette render is the dominant per-frame cost (`docs/tuning.md`). Cost is linear in palette count, because one encode is shared and only PLTE/tRNS are rewritten. z7 is about 75% of the pyramid.
- Hashing the full source array per frame is real cost: a 3500×7000 `f32` grid is about 98 MB. Use a fast SHA-256.
- Chunked classify caps temporaries at about 16 MiB (`2^21` cells × 8 bytes).
- Strided transparency probe (`[::8, ::8]`) short-circuits large regions.
- `f32` coordinate storage halves geometry memory.

---

## 16. Hazards and deliberate behaviors

Observed behaviors. Preserve them unless the port is explicitly allowed to change them.

- **H1. Identity depends on entry point.** Frame APIs hash each palette's dict; the `*_atomic` indexed and continuous wrappers hash the LUT ndarray. The same logical render through both produces different `palette_digest`, so a `PyramidIdentityConflict`. Do not mix entry points for one output root.
- **H2. Palette digest covers strings in the per-key table.** `unit` and any `_comment` inside a color key are digested. Editing them (for example `pm25._comment` in `classic.json`) changes identity, so a retry of a published frame raises `PyramidIdentityConflict` instead of `existing_valid`.
- **H3. Class numbering depends on the palette set.** `remap` and `K` come from the union over palettes passed. `model_remap` is in the policy digest. `frame_pyramid_identity_is_compatible` builds the model from whatever `color_tables` the caller passes, so callers must pass the same set. Changing `PALETTES` can change identities if the visible-class union changes. It does not for the shipped palettes.
- **H4. Integer vs float.** `1` and `1.0` hash differently (verified). A palette value changing from `5` to `5.0` changes identity.
- **H5. ASCII escaping.** `µ` hashes as `µ` (verified).
- **H6. PNG bytes matter only for adoption.** Identity is computed from inputs, and the marker's content digest is verified against the file's own bytes. Only **adoption** of a pre-v2 tree compares bytes with a fresh staging render, which needs identical zlib output. A Rust encoder cannot be expected to match, so Rust must not adopt unbound pre-v2 trees. It should raise and leave adoption to Python.
- **H7. Non-PNG files are outside identity.** `rglob("*.png")` ignores other files; adopted directories keep stray files (the adoption test leaves `stale-marker`).
- **H8. Legacy NaN coordinates raise.** `render_tiles` uses `lats.min()`, so `int(nan)` raises. The geometry path uses `nanmin` and tolerates NaN. Rust should tolerate NaN.
- **H9. Single-integer span returns None.** `_finite_index_bounds` returns None when `floor(min) == ceil(max)` after clamping, so a tile whose coordinates all land on one exact integer is dropped. Rare.
- **H10. Zero-weight invalid corners are ignored.** `sample_bilinear` excludes invalid corners from `valid_weight`. SciPy does not: `map_coordinates(order=1, cval=nan)` sampled at `(0.0, 0.5)` with NaN in row 1 gives NaN (verified). The continuous test uses SciPy as reference and passes, so its fixture never samples an exact integer next to NaN. Follow the implementation.
- **H11. Per-palette transparency inside shared tiles.** A tile is written for every palette if any class is nonzero, even if fully transparent for some palette. Counts stay equal.
- **H12. Documentation drift.** `ARCHITECTURE.md` describes staging as `<timestamp>.tmp-<uuid>`; the code uses `.<timestamp>.tmp-<hex>`.
- **H13. Palette name is a path component.** Unvalidated. `PALETTES` is operator-controlled (low risk). Rust should reject names containing `/`, `\`, or `..`.
- **H14. Geometry cache thrash.** Projected bilinear tiles cost 512 KiB; the default 256 MiB holds about 512. Larger frames evict and recompute every frame.
- **H15. Changing `TILE_PNG_COMPRESS_LEVEL` changes indexed identity** (`png_compress_level` is in indexed policy). Pin it across the fleet, or re-render of published indexed frames conflicts.
- **H16. Legacy counts can differ across palettes.** Only indexed and continuous guarantee equal counts.
- **H17. Lock held for the whole render.** Concurrent renders of one output set serialize; 1-in-64 stripe collisions serialize unrelated frames.
- **H18. Legacy tile compression is fixed at 1.** `render_tiles` ignores `TILE_PNG_COMPRESS_LEVEL`.
- **H19. Rename onto an empty directory succeeds** on POSIX and replaces it. Only non-empty targets give `EEXIST`/`ENOTEMPTY`. Linux `rename(2)` already does this.
- **H20. Marker link.** `os.link` needs hard-link support. Filesystems without it raise an uncaught `OSError`. Do not silently fall back to copy.

---

## 17. Test checklist (79 items)

Each item is an invariant to keep. Names come from `pytest --collect-only` in the reference environment.

### 17.1 `test_tiler.py` (3)

- [ ] `test_apply_color_table`: ranges `[5,20)` green and `[20,40)` red, `no_data_below=5`. Values 3 transparent, 10 green, 25 red, 50 transparent; shape `(2,2,4)`.
- [ ] `test_render_tiles_creates_files`: legacy render of 100×200 CONUS, zooms `[4,5]`, count > 0, paths `z/x/y.png`.
- [ ] `test_render_tiles_accepts_projected_source_grid`: EPSG:3857 source, zoom 4, tile 64, count > 0.

### 17.2 `test_tiler_atomic.py` (6)

- [ ] `test_atomic_render_publishes_final_dir_without_tmp_leftover`: final dir with PNGs, no `.{name}.tmp-*`.
- [ ] `test_atomic_render_adopts_exact_pre_marker_legacy_dir`: identical unmarked tree is adopted; `stale-marker` survives; marker written; `_fsync_directory_chain` called with `(out.parent, out.parent.parent)`; no staging.
- [ ] `test_atomic_render_rejects_visually_equal_but_byte_different_legacy_dir`: same pixels at compress 9 are not adopted; `PyramidIdentityConflict` matching `"cannot be adopted"`.
- [ ] `test_atomic_render_rejects_partial_pre_marker_legacy_dir`: one bogus tile is not adopted; same error; no marker.
- [ ] `test_atomic_render_transparent_frame_publishes_nothing`: count 0; no final; no staging.
- [ ] `test_atomic_render_cleans_tmp_on_failure`: render raises after partial writes; exception propagates; no final, no staging.

### 17.3 `test_tiler_overview.py` (8 items)

- [ ] `test_block_factors_track_mercator_pixel_size`: `(5,8)` at z4, `(2,4)` at z5, `(1,1)` at z7.
- [ ] `test_footprint_edges_cover_every_cell_once_and_flag_off_grid_pixels`: start `[0,0,3,6,9]`, stop `[0,3,6,9,10]`; descending axis gives the same cells.
- [ ] `test_footprint_edges_keep_the_cell_under_a_sub_cell_pixel`: pixel `[4.1,4.2]` on `arange(10)` gives `(4,5)`.
- [ ] `test_reduce_ranges_is_nan_aware_and_empty_ranges_are_nan`: `fmax` over `[1,nan,7,2,nan]` with ranges `[0,2) [1,4) [4,5) [5,5)` gives `1, 7, nan, nan`.
- [ ] `test_single_cell_core_survives_low_zoom_only_with_overview[legacy]`: one 62 dBZ cell is visible at z4 with `overview="max"` and not without; z7 matches.
- [ ] `test_single_cell_core_survives_low_zoom_only_with_overview[indexed]`: same, indexed.
- [ ] `test_overview_is_part_of_the_render_identity_only_when_set`: key present only when set; `"mean"` raises `ValueError`.
- [ ] `test_overview_rejects_categorical_grids`: overview with `category_map` raises `ValueError`.

### 17.4 `test_png_palette.py` (10 items)

- [ ] `test_replace_palette_matches_direct_encode[2]`, `[12]`, `[16]`, `[17]`, `[200]`: rewritten PNG decodes to the direct encode; IDAT and IHDR byte-identical; PLTE and tRNS match.
- [ ] `test_replace_palette_recomputes_crc`: every chunk CRC equals `crc32(type||payload)`.
- [ ] `test_replace_palette_inserts_trns_when_missing_and_drops_when_none`: tRNS inserted when alpha given (pixel 0 transparent); removed when `alpha=None`.
- [ ] `test_replace_palette_rejects_non_indexed_png`: RGBA input raises `ValueError` matching `"not indexed"`.
- [ ] `test_replace_palette_validates_lengths`: 4-byte rgb, over-long alpha, and bad signature raise `ValueError`.
- [ ] `test_iter_chunks_detects_truncation`: valid IEND yields `[b"IEND"]`; truncated input raises matching `"truncated"`.

### 17.5 `test_tiler_indexed.py` (52 items)

Palettes and class model:

- [ ] `test_all_shipped_palettes_share_bin_edges_per_color_key`: same edges per key across palettes; `n_classes == ranges + 1`; category names identical.
- [ ] `test_class_model_matches_legacy_for_every_shipped_table[float32]`: LUT lookup equals the legacy formula for every shipped table, at, just below, and just above every edge, plus NaN and inf.
- [ ] `test_class_model_matches_legacy_for_every_shipped_table[float64]`: same, f64 input.
- [ ] `test_class_model_handles_palettes_with_different_edges`: different edges per palette; values outside every range are class 0 in both.
- [ ] `test_class_model_later_overlapping_range_wins_like_legacy`: overlapping `[0,20)` and `[10,30)`; later wins.
- [ ] `test_categorical_model_matches_legacy`: categorical LUT equals legacy, including negative and unknown ids.
- [ ] `test_classify_is_chunked_and_shape_preserving`: 3000 rows; shape preserved; 1-D input; `(0,3)` gives `(0,3)`.
- [ ] `test_bilinear_samples_physical_values_with_explicit_nodata_policy`: mid-cell of four values gives 15.0; nodata corner gives NaN strict (`min_valid_weight=1`) and 10.0 partial (`0.75`).
- [ ] `test_fractional_indices_support_nonuniform_monotonic_axes`: `[-1,0.5,1.5,2.5]` ascending and `[3,1.5,0.5,-0.5]` descending; non-monotonic raises `"strictly monotonic"`.

Golden tiles and reference equality:

- [ ] `test_continuous_tiles_match_physical_bilinear_then_classify_reference`: classic tiles at z4–z6; decoded pixels equal LUT lookup of the scipy reference (f32 coords, order 1, `prefilter=False`, `cval=nan`) for every palette; counts equal.
- [ ] `test_categorical_tiles_equal_nearest_class_reference`: categorical tiles equal nearest-class reference (order 0, cval 0) for every palette.
- [ ] `test_indexed_tiles_share_idat_and_are_indexed_pngs`: colour type 3; same IDAT across palettes; PLTE and tRNS match the model; tile under `256*256` bytes.
- [ ] `test_legacy_renderer_is_a_marker_validated_rollback_path`: legacy writes RGBA; `CREATED`; `EXISTING_VALID` on retry; `ADOPTED` after marker deleted.
- [ ] `test_rollback_reuses_full_indexed_winner_without_claiming_legacy`: legacy request reuses indexed winners for two palettes; `existing_compatible`; bytes unchanged; identity still `indexed`.
- [ ] `test_foreign_renderer_rejects_changed_categorical_semantics`: indexed `{1: rain}` vs legacy `{1: snow}`: conflict matching `"incompatible"`.
- [ ] `test_foreign_renderer_rejects_unknown_algorithm_identity`: marker with renderer `future` and algorithm `unknown-v99`: conflict.
- [ ] `test_partial_foreign_renderer_set_fails_closed`: one indexed palette bound, second absent, legacy request: conflict matching `"partial palette set"`; second not created.
- [ ] `test_bound_identity_rejects_changed_render_inputs[source]`: different `source_id`: conflict.
- [ ] `test_bound_identity_rejects_changed_render_inputs[grid]`: lats shifted by 0.01: conflict.
- [ ] `test_bound_identity_rejects_changed_render_inputs[palette]`: one `rgba` channel XOR 1: conflict.
- [ ] `test_unknown_renderer_fails_closed`: `renderer="typo"` raises matching `"unknown tile renderer"`.
- [ ] `test_indexed_renderer_requires_one_named_canary_role`: missing canary raises `"CANARY_ROLE"`; canary `mrms` makes `mrms` indexed and `hrrr` legacy.
- [ ] `test_legacy_renderer_ignores_stale_canary_role`: `TILE_RENDERER=legacy` returns legacy for every role.
- [ ] `test_projected_continuous_grid_matches_physical_bilinear_reference`: EPSG:3857, tile 64, zooms `[4,5]`; every tile equals scipy reference through the inverse projection (requires pyproj).

Geometry cache:

- [ ] `test_geometry_cache_hits_on_second_frame_and_misses_on_new_grid`: first frame hits 0, misses > 0, entries == misses; second frame misses unchanged, hits == first misses; subsampled grid misses grow.
- [ ] `test_geometry_cache_skips_pyproj_on_cache_hit`: first frame transforms > 0 and `<=` misses; second frame transforms zero.
- [ ] `test_geometry_cache_is_bounded`: budget 0 and five puts leave one entry.
- [ ] `test_grid_spec_fingerprints_full_axes_not_only_shape_and_endpoints`: perturbed interior longitude with same endpoints is a miss.
- [ ] `test_geometry_cache_single_flight_computes_one_value`: four concurrent requests cause one compute, `waits == 3`, same object returned.

Atomic multi-palette publish:

- [ ] `test_atomic_multi_palette_publishes_each_pyramid_without_tmp_leftover`: both `created`; equal counts; marker schema 2, renderer `indexed`, palette name correct, digests present; no staging; opaque pixels equal LUT row 1.
- [ ] `test_publish_orders_tile_marker_and_parent_directory_durability`: order tiles, marker, rename; parent synced after rename; grandparent synced before (lock setup).
- [ ] `test_retry_finishes_parent_sync_after_post_rename_failure`: parent fsync `EIO` after rename; pyramid complete; retry `existing_valid`; parent fsync attempted exactly twice.
- [ ] `test_regular_file_fsync_errors_are_never_suppressed`: `EINVAL` from file fsync raises.
- [ ] `test_overlapping_palette_sets_share_a_publication_lock`: symlinked alias of the same final contends for the same lock.
- [ ] `test_invalid_overlapping_output_paths_fail_before_mutation[False]`: equal outputs raise `"distinct, non-nested"`; no output, no lock dir.
- [ ] `test_invalid_overlapping_output_paths_fail_before_mutation[True]`: nested outputs raise the same; no mutation.
- [ ] `test_publication_lock_root_must_be_a_strict_output_ancestor`: lock root equal to output raises `"lock root"`; output not created.
- [ ] `test_atomic_helpers_allow_palette_payload_supersets`: indexed and continuous atomic wrappers accept a LUT or model with more palettes than the output set.
- [ ] `test_atomic_multi_palette_rejects_incomplete_existing_dir`: existing dir with only a stale file raises `"no PNG tiles"`; other palette not created.
- [ ] `test_atomic_multi_palette_retry_reports_existing_valid`: second call `existing_valid` for both, counts equal.
- [ ] `test_completion_marker_detects_same_count_path_tamper`: renamed tile fails `is_complete_pyramid`; re-render raises `"completion path mismatch"`.
- [ ] `test_completion_marker_detects_tile_content_tamper`: flipped bit fails; re-render raises `"content mismatch"`.
- [ ] `test_exact_unmarked_pyramid_is_adopted_after_staged_rerender`: markers removed, identical content: both `adopted`; counts equal; complete.
- [ ] `test_partial_unmarked_pyramid_is_not_adopted`: marker removed, one tile deleted: `"cannot be adopted"`; no marker written.
- [ ] `test_schema_v1_marker_requires_exact_adoption`: v1 marker with bogus counts neither blesses nor blocks; exact match adopts; v2 written; v1 left in place.
- [ ] `test_schema_v1_marker_cannot_bless_a_partial_tree`: v1 marker plus missing tile: conflict; no v2 marker.
- [ ] `test_identity_marker_is_create_only`: different identity over an existing marker raises `"different identity"`; bytes unchanged.
- [ ] `test_atomic_partial_publish_converges_on_retry`: second rename `EIO`; classic stays; retry: classic `existing_valid`, muted `created`.
- [ ] `test_atomic_eexist_requires_winner_completion_marker`: `ENOTEMPTY` with a single-tile, unmarked winner: conflict matching `"differs"`; no staging.
- [ ] `test_atomic_eexist_exact_pre_marker_winner_is_adopted`: `ENOTEMPTY` with an exact unmarked winner: `adopted`; complete; no staging.
- [ ] `test_atomic_multi_palette_transparent_frame_publishes_nothing`: all-zero index: counts 0; all `empty`; no finals; no staging.
- [ ] `test_atomic_multi_palette_cleans_all_staging_on_failure`: render raises after writing staging; exception propagates; no finals; no staging.

---

## 18. Golden vectors

Reference outputs from the Python implementation. Rust canonical JSON and digests must reproduce these exactly.

| Value | Canonical JSON | `_stable_digest` (sha256 hex) |
|---|---|---|
| `{"tile_size": 256, "zoom_levels": [4, 5]}` | `{"tile_size":256,"zoom_levels":[4,5]}` | `438fa04ff498b1d035ccb67effc4be5f998faf71f43ef32fc0582fe76cb18fcf` |
| `1` | `1` | `6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b` |
| `1.0` | `1.0` | `d0ff5974b6aa52cf562bea5921840c032a860a91a3512f7fe8f768f6bbe005f6` |
| `float('nan')` | `{"float":"nan"}` | `c9772aeba1e20279e6937fa6a1ed65edca6e3e187fd1b2b2f7a3abb09cd02755` |
| `1e16` | `1e+16` | `a144838520595009e7daf5aff8472573f9ce6a4bcd8c30673675618883464ab0` |
| `1e-05` | `1e-05` | `5a9530d3c6d5faad6ba8afe16fb8e438df23865128370b2ec14ef622bb5f2ec9` |
| `0.01` | `0.01` | `312b95ee5a344d2f7a16ad817ff70788980da6e30b9bb04b651d53847abd476f` |
| `{"unit": "µg/m³"}` | `{"unit":"µg/m³"}` | `3a6376f348e82a364cb30a53071b3e4e1e49ae63ba72c3fa06fd8d501dd8c3ff` |
| `b"\x00\x01"` | `{"bytes":"0001"}` | `6cd21a7f60c406a723e34bad3b8d23951608a5b73d6756b7334083518739ea25` |
| `{"category_map": {2: "snow", 1: "rain"}}` | `{"category_map":{"1":"rain","2":"snow"}}` | `bab941ea5e130a853e11776b5192fd9627d3e0855b1500b0ffdaa6dcc061c9ad` |

- Array: `_array_content_digest(np.array([[1,2],[3,4]], dtype=np.uint8))` = `f190ff2f8cf085c85ca01972ceab76a3a8392250045f1015471552a9ef85b6df`. Hashed input: `{"dtype":"|u1","shape":[2,2]}` then bytes `01 02 03 04`.
- Path digest: `_tile_path_digest({"4/3/5.png", "4/3/6.png"})` = `5e456afeca265edaf43315d96b76dcf760778666ae3d5ee5882b7b1584e4e5c9`.
- Float32 cast: `np.float64(0.1).astype(np.float32)` = `0.10000000149011612`.

---

## 19. Rust design notes

### 19.1 Modules

| Module | Contents | Python source |
|---|---|---|
| `palette` | JSON parse preserving int/float; `ColorTable` (ranges or categories, `no_data_below` default -999, `unit`, `_comment` kept for digest); `load_palette`; env name list | `palettes.py`, JSON |
| `classify` | `ClassModel { edges: Option<Vec<f64>>, remap: Vec<u8>, luts }`; `build_class_model`, `build_categorical_class_model`, `collapse_transparent`, classify for f32, f64, and int | `ClassModel`, `build_*`, `_collapse_transparent` |
| `geometry` | tile math, `axis_to_fractional`, `finite_index_bounds`, `nearest_index`, `TileGeometry` (nearest, flat, bilinear with f32), `compute_tile_geometry`, footprint helpers | `_lat_lon_to_tile`, `_tile_bounds`, `_axis_to_fractional_indices`, `_compute_tile_geometry`, `_footprint_edges`, `_reduce_ranges`, `_max_block_factors` |
| `sample` | `sample_nearest`, `sample_bilinear` (exact arithmetic of 4.2), `footprint_max` | `sample_*`, `render_footprint_max_tiles` |
| `cache` | LRU bounded by bytes and entries; `None` cached; single-flight via `Mutex<HashMap<Key, Inflight>>` and `Condvar`; atomic stats | `_geometry_cache_*` |
| `png` | `encode_indexed` (bit-depth rule 7.1, zlib level), `replace_palette`, `iter_chunks`, `make_chunk` | `encode_indexed_png`, `png_palette.py` |
| `identity` | canonical JSON emitter (Python float repr, ASCII escape, sorted keys); `stable_digest`, `array_digest`, `grid_spec_digest`, `PyramidIdentity`, compatibility | `_stable_*`, `_array_content_digest`, `_grid_spec_digest` |
| `marker` | marker writer (exact key order), reader, `inspect_complete_pyramid`, path and content digests | `_write_completion_marker`, `_inspect_complete_pyramid` |
| `publish` | lock stripes (`flock`), staging names, fsync helpers, `durable_mkdir`, decisions of 13.2 and 13.5, execution of 13.6 | `_render_and_publish_atomic`, `_publication_group_lock`, `_fsync_*` |
| `render` | `render_indexed_tiles`, `render_continuous_tiles`, `render_frame_palettes` orchestration | same names |
| `env` | `bounded_int_env`, `tile_renderer_for_role`, constants | env helpers |
| `error` | `Invalid`, `Conflict(msg)`, `Unbound`, `Corrupt(msg)`, `Io(io::Error)` | exceptions |

### 19.2 Crates

- `png`: decoder for tests, and encoder for the first IDAT. Build later palette variants by chunk rewrite (7.3) with your own writer.
- `flate2`: zlib at level `PNG_COMPRESS_LEVEL`. **Bytes will not match zlib or Pillow.** Fine for identity (inputs only), not for adoption (H6). Consider the `zlib-rs` backend if byte parity matters.
- `crc32fast`; `sha2` (SHA-256 for identities, path digests, lock stripes); `blake2` (BLAKE2b, 16-byte output for fingerprints).
- `rayon`: tile-level parallelism within one frame.
- `proj`: EPSG:4326 to source CRS. Pin the PROJ version used in production, or accept ±1 ULP in `x`/`y` (which can flip a class on an exact edge).
- `nix` or `libc`: `flock`, `fsync`, `O_DIRECTORY`, `link`.
- `uuid` (or 128 random bits as hex): staging and temp names.
- `serde_json`: marker and palette files only. **Not** for digest canonicalization (its float output differs from Python `repr`).
- `ndarray` is optional. The data is 2-D with simple strides; slices or `Vec<f32>` with explicit `(h, w)` are simpler.

### 19.3 Required exactness

- Sampling arithmetic of 4.2: f32 storage, f64 weights, f32 accumulator stores in the stated order, f32 division. Test against the 240×480 synthetic fixture in `test_tiler_indexed.py` (`_synthetic_reflectivity`).
- Edge comparison in 5.3 (cast to f32 when strictly increasing after the cast).
- Axis mapping in 3.4 (affine formula and evaluation order).
- Bit-depth rule 7.1.
- Class numbering in 5.1 (ascending raw index, union over palettes). Required for identity parity (H3).
- Canonical JSON and digests in 10.1, only if Rust shares output roots with Python.
- Marker key order and separators in 11.

### 19.4 Decision the port must make

- **Option A: full identity parity.** Rust reproduces every digest in 10.1 and 10.2, uses `renderer = "indexed"` with the same algorithm, and shares output roots with Python. Mixed fleets converge on `existing_valid`. Byte equality is not required for identity. Pre-v2 adoption stays with Python (H6). **Recommended** if Rust replaces Python on the same roots.
- **Option B: separate identity.** Rust uses its own `renderer` name and digests. It must not share output roots with Python, or the publisher raises `PyramidIdentityConflict` ("complete palette set has mixed or incompatible render identities"). Simpler, but no rollback reuse.

### 19.5 Do not copy these

- Silent `EEXIST` handling outside the explicit branch of 13.6.
- A process-global `made_dirs`. It is per call in Python.
- Writing a marker into a final directory except through the exact-match `adopt` path.
- Leaving a tile with a nonzero class unwritten for any palette in the set.
