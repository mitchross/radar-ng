# Packed data tiles (v1, v2)

Radar animation is slow because the client fetches one colored PNG per tile per frame: about 1,100
requests for one loop, each a ~140 ms tunnel round trip (measured 2026-10-15). Packed tiles carry
many frames of raw quantized values in one image. The client colors them on the GPU and animates by
changing a uniform, with no network per frame. weather.com uses the same model (`/v2/tiler/packed`,
`timepacked`): 2 requests for its whole loop.

The colored PNG pyramids stay. Clients use packed tiles when a layer has `packed` metadata and fall
back to PNG rasters otherwise.

## Tile format

- PNG, 8-bit **RGB** (color type 2), no alpha, no gamma/ICC chunks. Each channel holds one frame
  (slot), so one pixel carries 3 slots. Alpha is avoided because decoders premultiply it.
- One tile covers one Web Mercator XYZ tile at `tile_size` = **512** px. A 512 tile at zoom `z`
  matches MapLibre's zoom `z` at 1:1.
- **Atlas layout:** slots are grouped into blocks of 3 (`block = slot / 3`, `channel = slot % 3`,
  R=0 G=1 B=2). Blocks are laid out row-major in a grid `cols` = 2 blocks wide, so the image is
  `512 * cols` wide and `512 * ceil(blocks / cols)` tall. With `max_slots` = 24 (8 blocks) the
  largest image is 1024 x 2048, within the WebGL2 minimum texture size.
- Pixel (px, py) of slot `s` is at atlas x = `(block % cols) * 512 + px`, y = `(block / cols) * 512 + py`.
- **Value:** `q` = 0 means missing or below the table's lowest value. Otherwise
  `value = offset + q * scale`. Unused slots are 0.
- **Sampling:** the same as the colored tiles: bilinear at native zooms, max-pooled footprint
  where source cells are finer than a pixel (`Overview::Max`), nearest for categorical layers.
- A tile whose every slot is 0 is not written.

## Quantization per palette key

| `color_key` (layers) | scale | offset | floor | coded range | palette edges not hit |
|---|---|---|---|---|---|
| `reflectivity` (radar, radar-composite, nowcast, radar-hrrr) | 2.5 | 0 | 5 dBZ | 5 to 637.5 dBZ | none |
| `temperature` (temperature, feels-like) | 1 | -80 | -79 °F | -79 to 175 °F | none |
| `dewpoint` (dewpoint) | 5 | -80 | -75 °F | -75 to 1195 °F | none |
| `wind_speed` (wind) | 5 | -5 | 0 mph | 0 to 1270 mph | none |
| `humidity` (humidity) | 10 | -10 | 0 % | 0 to 2540 % | 100.01 |
| `cape` (cape) | 500 | 0 | 500 J/kg | 500 to 127500 J/kg | none |
| `precip_accum` (precip-accum) | 0.05 | 0 | 0.01 in | 0.05 to 12.75 in | 20 |
| `cloud_cover` (cloud) | 5 | 0 | 10 % | 10 to 1275 % | 101 |
| `pm25` (air-quality) | 1 | 0.5 | 0 µg/m³ | 1.5 to 255.5 µg/m³ | 9.1, 20, 45, 90, 175, 500, 20000 |
| `ozone` (ozone) | 1 | 0 | 0 ppb | 1 to 255 ppb | 600 |

NaN or a value below `floor` stores 0. Otherwise `q = clamp(floor((value - offset) / scale), 1, 255)`.
The client decodes `value = offset + q * scale`, the bottom of the code's bin. A floor below
`offset + scale` stores the values in between as code 1: `precip_accum` 0.01 to 0.05 in is code 1
(0.05 in), which is in the same band. The manifest carries `scale` and `offset`; `floor` is
server-only.

- **Floor:** the lowest value any palette draws. Values below it are transparent anyway. On a real
  MRMS frame, clear-air noise below 5 dBZ made the tiles 15% larger.
- **Truncation, not rounding:** palette edges are multiples of `scale` from `offset`, so every value
  lands in the same color band as on the server's PNG tiles.
- **Coarsest exact step for model layers:** the palettes are stepped, and readouts use
  `/api/inspect`, not packed values. So each HRRR key uses the largest step that hits every band
  edge: the packed colors equal the PNG colors with the fewest bytes. Offsets are chosen so the
  lowest band starts at code 1. On a real HRRR run (20261015_22, f01 to f04), steps of 0.5 instead
  of 5 or 10 made the wind bucket 3.4x and the humidity bucket 4.6x larger. A smoother palette needs
  a finer step; clients read `scale` and `offset` from the manifest, so only the server changes.
- **Edges not hit:** a value at or above such an edge can decode into the band below it (or, past
  the coded range, into the top coded band). Each is listed in the table and checked by the tiler,
  web and app tests. They are out of physical range (humidity above 100.01 %, cloud above 100 %,
  more than 20 in of rain, ozone of 600 ppb or more) except `pm25`: its edges have a 0.1 µg/m³
  grid that 8 bits cannot cover up to 500. The step of 1 hits the AQI category edges 35.5, 55.5,
  125.5 and 225.5 exactly; the sub-band edges are off by up to 0.5 µg/m³, and 9.1 by 0.4.
- **Categorical layers** (`precip-type`) are not packed. Category ids 1 to 4 would fit in one
  code with nearest sampling, but the palette keys colors by name and v1 clients skip
  `categorical: true`. A later version can add the id-to-name map to the manifest.
- **2.5 dBZ step:** on a real full-CONUS MRMS frame (2026-10-15), one packed slot is 1.26 MB.
  0.5 dBZ steps give 2.09 MB, and 5 dBZ steps 1.05 MB. Packing takes about 100 ms with `png`
  `Compression::Balanced`; `High` saves 2% for twice the time.

## Typed codes (v2)

The MRMS layers color reflectivity by precipitation type when `PRECIP_TYPE` is on (default). Their
slots then hold typed codes, and the manifest says `"version": 2` with a `types` object:

```json
"types": {"stride": 32, "classes": ["rain", "snow", "mix"]}
```

- `code = class * stride + min(q, stride - 1)`, where `q` is the reflectivity code above. Code 0 is
  still missing. With `stride` 32, `q` <= 31 covers 5 to 77.5 dBZ, past the top palette band (75).
- A client decodes `class = code / stride` and `q = code % stride`, then colors `q` with that class's
  ramp. `no_data_below` applies to the rain ramp only, as on the server.
- **Compatibility:** v1 clients accept only `version` 1, so they draw a v2 layer from the PNG rasters,
  which are typed too. A v2 client accepts version 1 without `types` and version 2 with them.
- One bucket holds one code space. Its `.packed-bucket-v1.json` has `"types": 32` when it is typed.
  A frame whose encoding differs from its bucket's is not packed (raster path). When a layer's layout
  changes, the manifest drops the buckets of the old layout.
- A frame without a PrecipFlag file is packed as all rain.

### Precipitation type source

MRMS `PrecipFlag_00.00` (2-minute, same 0.01 degree CONUS grid). The worker takes the file nearest
the frame time, at most 5 minutes away. It lands about 20 s after the reflectivity file of the same
minute, so a fresh frame often takes the previous file. Flags (NSSL `UserTable_MRMS_PrecipFlags.csv`):

| Flag | Meaning | Class |
|---|---|---|
| 3 | snow | snow |
| 10 | cold stratiform rain (surface <= 5 °C) | mix within 25 km (25 cells) of snow, else rain |
| 1, 6, 91, 96 | warm stratiform, convective, tropical rain | rain |
| 7 | rain mixed with hail | rain: a warm-season liquid type; the reflectivity ramp shows hail cores |
| 0, -1, -3, other | none, missing, no coverage | rain (reflectivity decides what draws) |

PrecipFlag has no freezing-rain or sleet code, so mix is the rain-snow transition zone, not observed
ice. The rasters sample the class with the nearest cell at native zooms and the largest class of the
pixel's footprint at overview zooms (mix > snow > rain), the same as the packed slots.

## Buckets

A bucket is one directory of packed tiles: up to `max_slots` frames of one layer.

- **Observations** (`radar`, `radar-composite`): fixed 30-minute UTC windows. The id is the window
  start, e.g. `2026-10-15T18:30:00Z`. Each new MRMS frame takes the next slot of its window's
  bucket. The current bucket is rewritten on every frame; older buckets are final.
- **Runs** (`nowcast`, HRRR and AQM layers): one bucket per run, split into parts of `max_slots`
  frames. The id of part 0 is `<run>`; part `k` (k >= 1) is `<run>.p<k>`. For the nowcast, `<run>`
  is the anchor timestamp as it appears in the frame paths, e.g. `2026-10-15T18:30:38+00:00`. For
  HRRR and AQM it is the run id, e.g. `20261015_12`.
- **HRRR:** forecast hour `fhr` goes to part `(fhr - 1) / 24`: an 18 h run has one bucket, a 48 h
  run two. Hours render in parallel, so slots fill in completion order. Each layer of an hour is
  packed after its PNG render. An hour whose PNG tiles exist but whose slot is missing renders
  again.
- **AQM:** chunk `k` (messages `24k` to `24k + 23`) is part `k`: three buckets per 72-hour run.
- **Cost (2026-10-15, one machine, `tests/hrrr_packed_live.rs` and `tests/aqm_packed_live.rs`):**
  HRRR run 20261015_22, 18 hours, 9 packed layers: render 151 s, render and pack 208 s. Packing
  one hour takes about 0.5 s at slot 1 and 3 to 5 s near slot 18, because each new slot re-encodes
  the bucket's tiles. An 18-slot bucket is 1.7 MB (cape) to 22 MB (feels-like) over zooms 3 to 5;
  a z5 tile of temperature is about 420 kB. AQM run 20261015_12, 72 frames: pm25 20 s to 36 s,
  ozone 22 s to 42 s; buckets 27 MB (pm25) and 80 MB (ozone) per run, z5 tiles 110 kB and 330 kB.
  Smooth fields at 1-unit steps compress worse than palette PNGs: the win is requests, not bytes.
- A bucket's slot list only grows, and a slot never moves. Slots are normally in time order. An
  MRMS backlog is processed newest first, so a late frame can take a later slot than a newer frame.
  Clients look slots up by timestamp, never by position.
- A frame that is packed again (an activity retry) keeps its slot. A bucket that is full skips the
  frame, which then uses the raster path.

Layout on disk, under `TILE_DIR`:

```text
packed/<layer>/<bucket id>/.packed-bucket-v1.json   # {"v": n, "slots": [timestamps]}, written last
packed/<layer>/<bucket id>/<z>/<x>/<y>.png
```

Every tile file and the bucket file are written to a temporary name and renamed. The bucket file is
rewritten after all of that update's tiles are in place.

## Serving

`GET /tiles/packed/<layer>/<bucket>/<z>/<x>/<y>.png?v=<n>`

Packed tiles use the existing `/tiles` lane, because the public HTTPRoute sends only `/api`, `/tiles`
and `/basemap` to the tile server. The `/tiles` file server already maps this URL to the disk layout.

- The file, with `Cache-Control: public, max-age=86400, immutable` and `ACAO: *`. `v` is the bucket
  version from the manifest. The server ignores it, but it makes every bucket update a new URL for
  the CDN and the browser. A tile fetched with an older `v` can hold newer slots, which is harmless.
- A missing tile in a bucket that has `.packed-bucket-v1.json` returns `204` with the same headers.
  The bucket file is one of the `/tiles` completion markers, checked at the path minus `z/x/y.png`.
- Anything else returns the static lane's `no-store` 404.
- Bucket ids contain `:` and `+`. Clients may percent-encode them; the server decodes the path.

## Manifest

Each packed layer gets `layers.<layer>.packed`:

```json
{
  "version": 1,
  "tile_size": 512,
  "min_zoom": 3,
  "max_zoom": 6,
  "cols": 2,
  "max_slots": 24,
  "color_key": "reflectivity",
  "categorical": false,
  "scale": 0.5,
  "offset": -32.0,
  "url_template": "/tiles/packed/{layer}/{bucket}/{z}/{x}/{y}.png?v={v}",
  "buckets": [
    {"id": "2026-10-15T18:30:00Z", "v": 7, "slots": ["2026-10-15T18:30:38+00:00", "..."]}
  ]
}
```

- Slot timestamps use the same strings as the layer's `frames[].timestamp`. A client finds a frame's
  `(bucket, slot)` by timestamp.
- Zooms: MRMS layers use 512-px zooms 3 to 6 (the 256-px pyramid's 4 to 7). Nowcast, HRRR and AQM
  layers use 3 to 5. Clients overzoom past `max_zoom`.
- The manifest is updated under `manifest.lock` after the bucket file. Buckets drop out of the
  manifest before their directories are deleted, as frames already do.
- Retention: an MRMS bucket goes when its whole window is older than the MRMS retention (4 h). The
  nowcast lists only the current run's buckets; other run buckets are deleted after 60 minutes.
  HRRR and AQM delete the buckets of runs the manifest no longer references, with the same
  retention as their PNG run directories.
- **Run layers (HRRR, AQM):** the publish writes the frames and `packed` in one manifest write. A
  layer lists the buckets of every run its frames come from; an HRRR layer can show the extended
  tail of an older 48 h run. A slot whose frame the layer does not show from that run (a newer run
  replaced it, or the hour is not published yet) is listed as `""` in place, so slot positions
  stay and each frame timestamp is in at most one bucket. A bucket with no shown slot is not
  listed. When packing is off, the publish removes `packed` before it writes the new frames.
- `PACKED_TILES=0` stops writing buckets (`docs/configuration.md`).

## Coloring (clients)

The client builds a 256 x 1 RGBA lookup table per (palette, `color_key`) from `/api/palettes`. For
`q` = 1..255 it computes `value = offset + q * scale` and applies the renderer's rules:

- `value < no_data_below` (default -999): transparent.
- Ranges are half-open `[min, max)`, and a later overlapping range wins.
- A value in no range: transparent.

`q` = 0 is always transparent. Categorical layers (HRRR precip type) are not packed. For typed
codes (v2), build the table per code: class `code / stride` picks `ranges` (rain) or
`types.<class>.ranges` of the palette key, and `q = code % stride` gives the value. Clients
color every layer through the table of its `color_key`; the built-in fallback covers only
`reflectivity`, so other layers use rasters until `/api/palettes` answers.

The shader reads the slot's channel with `texelFetch` (nearest), maps it through the table, and
multiplies by the layer opacity. A later version may crossfade two slots.

## Clients

- **Web:** a MapLibre custom layer (WebGL2) under the label layers. It draws one quad per visible
  tile. It fetches every bucket the playback sequence needs, for the tiles in view at
  `clamp(floor(zoom), min_zoom, max_zoom)`, and keeps parent tiles while children load.
  Frames without a packed slot use the existing raster path.
- **App:** a Skia overlay with an SkSL `RuntimeEffect`, using the camera projection the wind
  overlay uses. Atlas and table are image shaders.

## Rollout

1. Server: produce packed buckets for `radar` and `radar-composite` (MRMS) and `nowcast`; serve
   `/packed`; manifest metadata. PNG pyramids unchanged.
2. Web and app: packed renderer behind the manifest check, raster fallback. Off switch in the URL
   for web (`?raster=1`) and in settings for the app.
3. HRRR layers (not `precip-type`) and AQM layers: done. Then the new layers.
