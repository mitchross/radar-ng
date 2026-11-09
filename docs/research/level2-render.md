# Polar sweeps to map tiles

Decision for [How do polar sweeps become map tiles?](https://github.com/mitchross/radar-ng/issues/275), researched 2026-11-08 (US Eastern). Repo inspected at `d78b02a`. Research only; no product implementation.

## Recommendation

Keep the QC'd numeric polar sweep. Inverse-map only requested 256-pixel XYZ tiles into it, then cache immutable numeric tiles and palette PNGs. Do not grid every sweep over a whole station before anyone asks for tiles. Do not pre-render a station's complete z9–13 pyramid.

Use nearest gate-footprint ownership as the truthful baseline. Add bounded subpixel coverage sampling for minification and gate edges. Keep smoothing separate from anti-aliasing. Treat a Cartesian intermediate as a later, measured optimization for heavily reused areas or scientific mosaics, not a prerequisite for display.

This matches the map's viewed-stations-only policy and preserves the original sampling geometry. It requires a new polar sampler and a lazy tile/publication contract; today's `render_frame_palettes` is not an on-demand polar renderer.

## What upstream projects actually do

| Project | Inspected path | Finding |
| --- | --- | --- |
| Supercell Wx | [Level II view, pinned commit](https://github.com/dpaulat/supercell-wx/blob/548ffc41727eb5f9d5e10e929d213ed9de7babac/scwx-qt/source/scwx/qt/view/level2_product_view.cpp) | `ComputeSweep` builds two triangles per gate. Flat mode repeats the gate moment; optional smoothing assigns neighboring moments to vertices. It does not require a Cartesian meteorological grid. |
| Py-ART display | [RadarMapDisplay](https://github.com/ARM-DOE/pyart/blob/main/pyart/graph/radarmapdisplay.py) | `plot_ppi_map(edges=True)` obtains gate-edge coordinates and passes them with masked data to `pcolormesh`. `raster=True` rasterizes vector output; it is not objective-analysis gridding. |
| Py-ART analysis | [grid_from_radars](https://arm-doe.github.io/pyart/API/generated/pyart.map.grid_from_radars.html), [map_gates_to_grid](https://arm-doe.github.io/pyart/API/generated/pyart.map.map_gates_to_grid.html) | A separate operation maps gates to a Cartesian grid using a radius of influence. Barnes2, Barnes, Cressman and nearest are analysis choices. The grid changes the field's support/interpolation, rather than merely drawing original gate cells. |
| NOAA WCT | [official project](https://www.ncei.noaa.gov/products/weather-climate-toolkit), [4.9.1 source ZIP](https://www.ncdc.noaa.gov/wct/app/dist/wct-4.9.1-src.zip) | `wct/src/gov/noaa/ncdc/wct/decoders/cdm/RadialDatasetSweepRemappedRaster.java`, lines 634–743, loops over output lat/lon pixel centers, computes station-relative distance/azimuth, looks up a ray and gate, and preserves missing/out-of-range samples. Optional smoothing happens later. This is direct output-to-polar sampling. |

Use these as architectural evidence, not performance comparisons. Supercell Wx is a desktop GPU renderer, Py-ART is a plotting/analysis library, and WCT's inspected path produces a geographic raster, not an XYZ endpoint.

Projection caveat: Supercell Wx's inspected coordinate builder passes gate distance into a geodesic forward calculation. WCT's inspected remapper uses projected ground distance directly for its gate index and computes beam height separately. Neither is the reference for correct slant-to-ground placement. Use Py-ART's explicit propagation transform instead.

Missing-data caveat: Supercell's smoothed boundary moments can be remapped to an edge value. WCT's optional smoothing converts dBZ to linear Z and convolves after substituting a baseline for missing samples. Do not copy either boundary policy silently. [Matplotlib pcolormesh](https://matplotlib.org/stable/api/_as_gen/matplotlib.pyplot.pcolormesh.html) defaults to `antialiased=False`; masked cells become transparent. None of these observations proves footprint-aware minification is solved for radar-ng.

## Geometry: slant range is not ground distance

Reference: [Py-ART antenna transforms](https://github.com/ARM-DOE/pyart/blob/main/pyart/core/transforms.py), `antenna_to_cartesian`, implementing the standard-atmosphere 4/3-Earth model.

Let `r` be slant range in metres, `e` elevation, `a` clockwise azimuth from north, `Re = 6,371,000 m`, and `K = 4 Re / 3`. Use radians inside trig functions.

```text
h = sqrt(r² + K² + 2 r K sin(e)) - K
s = K asin(r cos(e) / (K + h))
x = s sin(a)
y = s cos(a)
```

`h` is beam-center height above the radar reference, not above local terrain. Add station antenna altitude for an approximate MSL height. Terrain-relative height needs terrain data. Abnormal refraction, beam blockage and the finite vertical beamwidth remain real uncertainties.

Map `(x,y)` via a station-centered geographic transform using the geographic Earth model, not `K`. Then project lat/lon to EPSG:3857. Py-ART exposes spherical azimuthal equidistant and optional ellipsoidal geographic transforms. For tile inverse mapping, first obtain geographic ground distance `s` and bearing `a` from station to pixel. For a fixed sweep elevation, the derived inverse is:

```text
alpha = s / K
r = K sin(alpha) / cos(e + alpha)
```

This follows from `tan(alpha) = r cos(e) / (K + r sin(e))`. Reject nonphysical/out-of-coverage solutions. The Rust probe round-tripped six ranges through 460 km at 0.5° within 1 micrometre. That checks the algebra, not the atmosphere or geodetic model. Varying per-ray elevations require ray-specific inversion and cache identity, not blindly using the nominal tilt.

Use each ray's actual azimuth, elevation, first-gate range, gate spacing and gate count. Derive cell boundaries halfway between adjacent centers, respecting 359°/0° wrap. Keep true missing sectors open; never stretch a neighboring ray across a gap or join incomplete sweep ends. Handle different moment geometries explicitly. A regular `[azimuth,range]` array is a storage convenience, not a geographic rectilinear grid.

### Why high zoom does not mean street-scale observations

The following are calculated for 0.5° elevation. The 0.5° angular column is illustrative ray spacing; 1° is an illustrative beamwidth, not a guarantee for every sample. Read geometry from the data. [NOAA's radar overview and ICD links](https://www.ncei.noaa.gov/products/radar/next-generation-weather-radar) provide the operational format context.

| Slant range km | Ground arc m | Beam center m above radar | 0.5° cross-range spacing m | 1° beam footprint m, approximately |
| --- | ---: | ---: | ---: | ---: |
| 25 | 24,998 | 255 | 218 | 436 |
| 50 | 49,995 | 583 | 436 | 873 |
| 100 | 99,981 | 1,461 | 873 | 1,745 |
| 150 | 149,956 | 2,633 | 1,309 | 2,617 |
| 230 | 229,881 | 5,119 | 2,006 | 4,012 |
| 460 | 459,317 | 16,453 | 4,008 | 8,017 |

Never label 250 m range-gate spacing as uniform ground resolution. Even close to the radar, angular sampling and beam volume matter. At long range, height and beam footprint dominate. z13 pixels can display gate boundaries smoothly; they cannot create new information. Choose the MRMS crossover using range, age, beam height and QC, not zoom alone. Do not equate an empty or filtered gate with a healthy clear-air measurement.

## Sampling and anti-aliasing policy

1. Apply QC and coverage masks in numeric space before palette lookup. Preserve no-data, rejected gates, range-folded codes, missing rays and below-display-threshold values as distinct states internally.
2. For magnification, sample the containing gate footprint. Optional physical-value interpolation must be an explicit display mode, never interpolation between palette indices or colors. Do not call smoothing higher resolution.
3. For minification, estimate pixel footprint against radial and angular cell sizes. Start with a bounded 2×2 subpixel rule; compare against a denser or area-integrated reference near the radar, thin echoes and sharp boundaries. Four taps are a candidate, not proof of alias-free output.
4. If averaging reflectivity, average linear `Z = 10^(dBZ/10)` and convert back. Record this as a display aggregation choice. Do not silently replace it with max pooling: maximum preserves peaks but biases area/intensity upward. Keep the unsmoothed native sampler as the test reference.
5. Use strict valid-support masking initially. Do not interpolate across rejected gates or sectors. Any later partial-coverage/alpha rule must specify its valid-weight threshold and preserve the missing mask. A transparent Level II hole must reveal MRMS intentionally, not imply zero precipitation.
6. Derive sample positions from global XYZ pixel coordinates, including subpixel offsets. If a filter needs neighbors, render a halo and crop. Adjacent tiles must match across edges and between palettes.

## CPU and memory estimates

### What was actually run

A dependency-free Rust microprobe ran in the Hermes x86_64 pod with `rustc 1.99.1 -O`. This is separate from the product's pinned Rust toolchain. It allocated a synthetic 720-ray × 1,840-gate f32 field, then inverse-mapped one 256×256 tile per zoom near 40°N, 100°W, roughly 50 km east and 30 km north of the origin. Each geometry timing averaged 12 iterations. It used spherical geographic inverse distance/bearing, the derived slant-range inverse, uniform synthetic rays and nearest gate indices. A hot gather traversed those indices 1,000 times. Shared-host timing varied noticeably between two runs.

No NOAA volume was decoded. These are synthetic compute measurements and capacity estimates, not real-storm render throughput, cold-start latency, QC cost or a production SLO result. The smooth synthetic field is deliberately not a compression prediction for actual weather.

| z | Ground m/pixel at 40°N | Tile side km | Cold geometry, 1 tap ms | Cold geometry, 4 taps ms | Hot 1-tap gather ms | Synthetic PNG bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 9 | 234.217 | 59.959 | 11.272 | 47.291 | 0.059 | 10,909 |
| 10 | 117.108 | 29.980 | 11.434 | 44.471 | 0.056 | 6,812 |
| 11 | 58.554 | 14.990 | 11.965 | 53.689 | 0.058 | 4,750 |
| 12 | 29.277 | 7.495 | 16.480 | 47.052 | 0.082 | 3,266 |
| 13 | 14.639 | 3.747 | 14.792 | 113.570 | 0.097 | 2,529 |

Ground scale uses `2 pi 6378137 cos(latitude) / (256 × 2^z)`. Geometry dominates the cold path; zoom does not change output pixel count. Cache geometry across scans only when the complete geometry identity matches. Four-tap timings include four complete transforms, not four cheap neighboring field loads.

For planning, allow roughly 10–20 ms cold one-tap geometry or 45–120 ms naive four-tap geometry per tile on this shared host, plus sampling, aggregation, classification, PNG and I/O. Hot one-tap numeric gather alone was about 0.05–0.10 ms, not total render time. The synthetic Python zlib stage measured medians of 3.204/2.105/1.441/1.251/0.896 ms at z9–13 over 40 iterations. It encoded 8-bit indexed PNG, no row filter, zlib level 6, full PLTE/tRNS; radar-tiler's adaptive filters and class model differ.

Per-tile raw planes are zoom-independent: 65,536 bytes of u8 indices, 262,144 bytes of f32 samples, or 262,144 bytes for a production u32 gate-index map. Four u32 index maps cost 1,048,576 bytes before weights/masks. The throwaway prototype used native `usize` indices and therefore allocated twice the proposed u32 map size on x86_64. Cap this cache by bytes, not just tile count. Compressed PNG size is data-dependent; budget at least the raw indexed plane plus PNG overhead for noisy fields, not the small synthetic examples.

### Per sweep

The illustrative 720×1,840 sweep has 1,324,800 gates extending nominally to 460 km at 250 m spacing. It is not a promise that all scans/moments have this shape.

| Retained data | Bytes per sweep, excluding metadata/allocator overhead |
| --- | ---: |
| u8 raw codes | 1,324,800 |
| u16 raw codes | 2,649,600 |
| f32 reflectivity | 5,299,200 |
| f32 reflectivity + u8 validity/QC | 6,624,000 |
| Four f32 moment planes + one u8 mask | 22,521,600 |

Synthetic allocation and trigonometric field initialization took 23.031 ms. **This is not a Level II decode benchmark.** Compression, decoder, selected cuts, QC and real input sizes are owned by the decoder/feed research and must be measured before sizing the worker. Cache short-lived decoded sweeps per station/scan and bound stations, frames and in-flight work. Do not multiply these numbers by every national radar.

A 460 km-wide Cartesian square (230 km radius) costs 13,542,400 bytes for one f32 field at 250 m, 54,169,600 at 125 m, or 338,560,000 at 50 m, excluding masks, geometry and temporary arrays. A square covering the full illustrative 460 km radius is larger still. Rendering the 250 m square by the same inverse transform would visit 3,385,600 pixels: about 0.57 CPU-seconds at an assumed 11 ms per 65,536 pixels. That is an extrapolation, not a measured Barnes gridding benchmark. It then needs another tile resampling pass and does not preserve 15 m z13 information.

The simple geometry-only break-even is about 52 unique tiles per matching sweep geometry versus that 250 m square. It excludes reuse across scans, QC, different output resolution, cache misses, interpolation and I/O. Measure real unique-tile reuse before adopting it as a switch threshold.

### Why not render the whole station pyramid?

For an illustrative **230 km ground-radius** disk at 40°N, `pi R² / tile_side²` gives approximate interior tile equivalents. These are not exact intersecting-tile counts; boundary tiles and latitude variation add overhead.

| z | Disk tile equivalents | Geometry-only CPU s at assumed 11 ms/tile | Uncompressed u8 tile planes MiB |
| --- | ---: | ---: | ---: |
| 9 | 46.2 | 0.508 | 2.89 |
| 10 | 184.9 | 2.034 | 11.56 |
| 11 | 739.6 | 8.136 | 46.23 |
| 12 | 2,958.5 | 32.543 | 184.91 |
| 13 | 11,834.0 | 130.174 | 739.62 |

Those costs are per frame and exclude encode/storage. A viewed tile still costs one tile, regardless of station coverage. On-demand work avoids paying for thousands of unseen high-zoom tiles; palette variants can share sampled values/class indices and IDAT where compatible.

## Fit into radar-ng

Repo links below are pinned to the inspected revision, not future promises.

- [radar-tiler README](https://github.com/mitchross/radar-ng/blob/d78b02a/rust/crates/radar-tiler/README.md), [geometry.rs](https://github.com/mitchross/radar-ng/blob/d78b02a/rust/crates/radar-tiler/src/geometry.rs): `SourceGeometry` currently handles regular geographic axes and projected Cartesian axes. There is no polar source kind. Do not pretend azimuth/range are latitude/longitude or pass an irregular gate mesh into that API.
- [render.rs](https://github.com/mitchross/radar-ng/blob/d78b02a/rust/crates/radar-tiler/src/render.rs): `render_frame_palettes` enumerates a frame pyramid and publishes it under a file lock. Add a bounded tile-level polar sampler; reuse class models, palettes, indexed PNG encoding, identities and cache accounting. Preserve the existing grid path. The current max-overview route only accepts regular lat/lon grids, so it cannot be reused unchanged for polar footprints.
- [sample.rs](https://github.com/mitchross/radar-ng/blob/d78b02a/rust/crates/radar-tiler/src/sample.rs): physical bilinear sampling already carries explicit valid-weight semantics. Reuse the discipline, not the rectilinear coordinate assumptions. Do not interpolate classified indices.
- [API manifest route](https://github.com/mitchross/radar-ng/blob/d78b02a/rust/crates/radar-api/src/manifest.rs): reads via `radar_core::manifest`, retains last good state, caches for 15 seconds and computes ETags. Never decode or render inside manifest reads.
- [manifest types](https://github.com/mitchross/radar-ng/blob/d78b02a/rust/crates/radar-core/src/manifest.rs): frames are unique by timestamp within a layer; `extra` preserves metadata. Station frames at the same time need station-scoped layers or a deliberately extended model. Do not allow one station to replace another merely because times match.
- [tile route](https://github.com/mitchross/radar-ng/blob/d78b02a/rust/crates/radar-api/src/basemap.rs): currently serves filesystem pyramids; missing tiles under a complete marker become a transparent response. A lazy tile miss is **not** known-empty. Add a separate explicit lazy route/contract; never mark an unrendered region complete and let this handler hide it.

Proposed flow: viewport demand → bounded/single-flight station decode + QC → immutable sweep artifact verified → tile workers inverse-sample requested XYZs → numeric tile cache → palette PNG cache → server URL used by both clients. Keep heavy work off the API async executor. Bound concurrent blocking jobs and maintain Temporal heartbeats; a timeout does not cancel `spawn_blocking` work.

Key caches by station, exact input/scan/sweep identity, actual ray geometry, field, QC version, propagation/geodetic model, sampler/AA version, tile size, XYZ and palette/quantization identity as applicable. Deduplicate simultaneous misses. Use immutable scan paths, not mutable `latest` URLs. Retain old objects long enough for last-good manifests and CDN/client reads.

Publication needs an explicit decision in the build spec: the [north-star manifest SLO](https://github.com/mitchross/radar-ng/blob/d78b02a/docs/reference/north-star.md) says required palettes/zooms must exist before advertising a frame. Recommended lazy mode should advertise a verified renderable numeric source with explicit readiness semantics, not falsely claim a precomputed pyramid is complete. Until that contract and its tests exist, publish only a bounded fully rendered viewport product under the existing completeness rules. RustFS versus PVC placement and final manifest naming belong to the map's remaining storage/schema decision, not this rendering research.

Both web and app consume our server tiles only. Both must implement the same range/QC/age-aware MRMS cross-fade, station/tilt/scan-time provenance, missing-coverage behavior and last-good fallback. Never substitute a stale station silently for a fresh observation. App changes need a native rebuild. Zooms 9–13 are the comparison range, not an endorsement of a universal crossover at z9.

## Build gates and remaining risks

- Compare geolocation with Py-ART reference gates at near/far ranges, cardinal bearings, azimuth wrap, negative/variable tilts and station altitude. Validate spherical versus ellipsoidal error against an explicit pixel-error budget.
- Verify gaps, QC holes, varying gate counts, missing dual-pol moments and repeated lowest-elevation cuts. Do not invent gates during interpolation.
- Add tile-seam, palette parity, sparse-echo minification and all-transparent tests. Compare bounded AA against a high-quality reference and benchmark its worst case.
- Measure actual decoded sweeps, QC, source bytes, worker peak RSS, tile encode size and CPU on target worker hardware. The synthetic figures cannot size cold downloads or prove a 100-user SLO.
- Test stampedes, station lateness, cancelled requests, object-store delay, manifest/retention races and last-good fallback on both clients.
- Resolve lazy readiness versus current complete-pyramid semantics before shipping. Full-station pre-rendering is not an acceptable workaround for this on-demand design.

## Reproduction record

Commands actually executed (all probe files stayed under scratch):

```sh
rustc -O /opt/data/cache/scratch/level2-render-probe.rs -o /opt/data/cache/scratch/level2-render-probe
/opt/data/cache/scratch/level2-render-probe > /opt/data/cache/scratch/level2-render-probe.log
python3 /opt/data/cache/scratch/level2-render-bytes.py > /opt/data/cache/scratch/level2-render-bytes.log
```

The probe generates `30 + 20 sin(ray × .03) cos(gate × .009)` dBZ. It computes haversine ground distance and forward bearing per pixel/subpixel, applies the inverse above, rounds to the nearest of 720 synthetic rays and floors slant range / 250 m. The test shape starts at range zero; production must instead use decoded gate-center/edge metadata. It writes 256×256 u8 planes quantized as `round(2 × dBZ) + 1`. PNG measurement prepends filter-zero scanlines and zlib-compresses at level 6, with a grayscale 256-entry palette and transparent index zero. No synthetic results are represented as NOAA observations.

Evidence was checked using raw upstream source downloads as well as documentation extraction. The NOAA source was downloaded and inspected from the official archive. No product files changed, so Rust/web/app gates were not run; this branch contains research only.
