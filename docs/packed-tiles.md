# Packed data tiles (v1)

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

| `color_key` | scale | offset | floor | coded range |
|---|---|---|---|---|
| `reflectivity` (radar, radar-composite, nowcast, radar-hrrr) | 2.5 | 0.0 | 5 dBZ | 5 to 637.5 dBZ |
| `temperature` | 1.0 | -80.0 | none | -79 to 175 °F |
| `wind_speed` | 0.5 | 0.0 | none | 0.5 to 127.5 mph |
| `precip_accum` | 0.05 | 0.0 | 0.01 in | 0.05 to 12.75 in |
| `cloud_cover` | 0.4 | 0.0 | 10 % | 10 to 102 % |

`q = clamp(floor((value - offset) / scale), 1, 255)`. NaN, a value below `floor`, or `q < 1` before
the clamp stores 0. The client decodes `value = offset + q * scale`, the bottom of the code's bin.

- **Floor:** the lowest value any palette draws. Values below it are transparent anyway. On a real
  MRMS frame, clear-air noise below 5 dBZ made the tiles 15% larger.
- **Truncation, not rounding:** palette edges are multiples of `scale` from `offset`, so every value
  lands in the same color band as on the server's PNG tiles.
- **2.5 dBZ step:** on a real full-CONUS MRMS frame (2026-10-15), one packed slot is 1.26 MB.
  0.5 dBZ steps give 2.09 MB, and 5 dBZ steps 1.05 MB. Packing takes about 100 ms with `png`
  `Compression::Balanced`; `High` saves 2% for twice the time.

## Buckets

A bucket is one directory of packed tiles: up to `max_slots` frames of one layer.

- **Observations** (`radar`, `radar-composite`): fixed 30-minute UTC windows. The id is the window
  start, e.g. `2026-10-15T18:30:00Z`. Each new MRMS frame takes the next slot of its window's
  bucket. The current bucket is rewritten on every frame; older buckets are final.
- **Runs** (`nowcast`, HRRR layers): one bucket per run, split into parts of `max_slots` frames.
  The id of part 0 is `<run>`; part `k` (k >= 1) is `<run>.p<k>`. For the nowcast, `<run>` is the
  anchor timestamp as it appears in the frame paths, e.g. `2026-10-15T18:30:38+00:00`.
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
- `PACKED_TILES=0` stops writing buckets (`docs/configuration.md`).

## Coloring (clients)

The client builds a 256 x 1 RGBA lookup table per (palette, `color_key`) from `/api/palettes`. For
`q` = 1..255 it computes `value = offset + q * scale` and applies the renderer's rules:

- `value < no_data_below` (default -999): transparent.
- Ranges are half-open `[min, max)`, and a later overlapping range wins.
- A value in no range: transparent.

`q` = 0 is always transparent. Categorical layers (precip type) are not packed in v1.

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
3. HRRR and AQM layers, then the new layers.
