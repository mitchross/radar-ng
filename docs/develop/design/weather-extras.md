# Weather extras: winter radar, snow, star map, nice day, sky watch

Build spec for [#314](https://github.com/mitchross/radar-ng/issues/314) plus two additions:
a star forecast map and winter mode. This note implements nothing. Every build PR targets the
integration branch `feat/weather-extras`. One final `feat/weather-extras → master` PR is left for
Mitch.

Sources were checked with live requests on 2026-11-17. Sizes and latencies are from that one day,
not a published SLA. Anything not checked is marked **not verified**, and the ticket that uses it
must check it first.

## Recommendation

Build winter first, in this order: ice colours on today's radar, the same colours on the future
frames, snow and ice forecast layers, snow on the ground, snow chance, then the "snow at your spot"
card and the "snow starts in ~N min" notice. Everyday cards and the sky features follow in
parallel lanes.

Rules that hold for every slice:

- Our server is the only host the clients call. Public data is fetched and cached server-side.
  No API keys. No third-party tiles.
- Every user-facing slice ships web and app in one PR. Shared logic lives in `web/src/shared`
  and the app re-exports it.
- New raster layers reuse the existing path: grid → palette → PNG pyramid → numeric grid →
  manifest. A new layer needs no new API route ([add-layer](../../../.claude/commands/add-layer.md)).
- A layer with no data shows an honest empty state, never a blank map or a spinner that never ends.

## 1. What already ships

Checked at `d96e36f`.

| Area | Today | Gap |
|---|---|---|
| Observed radar colours | MRMS radar is coloured rain / snow / mix. MRMS PrecipFlag code 3 is snow. Mix is a heuristic: cold stratiform rain within about 25 km of snow (`rust/crates/radar-ingest/src/precip_type.rs`). | No ice class. PrecipFlag has no freezing-rain or sleet code. |
| Packed tiles | Value is `class*32 + dBZ step`, classes 0–2 (`rust/crates/radar-tiler/src/typed.rs`). | Classes above 2 are rejected. |
| Future frames | Nowcast, HRRR (`radar-hrrr`) and GFS (`radar-gfs`) draw plain rain colours. | Colours change at "now". |
| HRRR precip type | CRAIN / CSNOW / CFRZR / CICEP decode and a standalone `precip-type` PNG layer exist. | Layer is off in the deployment and not in any picker. No numeric grid. |
| Snow amounts | None. Open-Meteo syncs snow water equivalent, but the point API does not request snowfall. | No snowfall, snow depth, ice or WPC layer. No snow card. |
| Astronomy | Sunrise and sunset come from Open-Meteo. The "moon" is only a clear-night icon. | No moon phase, planets, ISS or star forecast. |
| Normals, indices, holidays | None. | All new. |

The old branch `feat/precip-type-radar` is fully merged into master (commit `33c29e9` is the
rain/snow/mix work). There is nothing left on it to reuse.

## 2. Where a first-time visitor finds each feature

The web has no tabs: it is a full-screen map with a forecast panel. The app has Home, Nowcast,
Radar, Alerts and Settings.

| Feature | Web | App |
|---|---|---|
| Rain / snow / mix / ice radar, past and future | Default radar. Nothing to turn on. Legend shows four ramps. | Same on Radar tab and the Home mini map. |
| Snow forecast, Ice forecast, Snow depth, Snow last 24 h, Snow chance | New **Winter** group in the layer sheet, above Severe. | New **Winter** group in the map options sheet. |
| Snow at your spot | Forecast panel card, placed right after alerts. Shows only when snow is forecast in the next 72 h or is on the ground. | Home card in the same position. |
| "Snow starts in ~N min" | The existing next-hour notice changes its words. Same opt-in as the rain notice. | Same, on Home and Nowcast. Same local notification opt-in. |
| Nice Day and Bonfire index | Forecast panel card under the hourly strip. | Home card under the hourly strip. |
| Nice Day map | **Nice Day** in the main layer list. | Same. |
| Countdown to the next holiday, with sunset | Small forecast panel card near the 7-day list. | Home card near the 7-day list. |
| Temp vs normal | **Temp vs normal** in the main layer list, and one line on the current-conditions block. | Same layer, and a line on the Home hero. |
| White Christmas chance | Seasonal forecast panel card, Nov 15 to Dec 25 only. | Seasonal Home card. |
| Look up tonight (ISS, planets, moon) | Forecast panel card, shown from two hours before sunset until sunrise. A "Tonight" link shows it at any time. | Home card with the same rule. |
| Star forecast map | **Stargazing** in the main layer list. The Look up tonight card links to it. | Same. |

Winter needs no setting. When the snow card is showing, the Winter group opens first in the
layer sheet.

## 3. Winter radar

### 3.1 Ice on observed radar

MRMS cannot tell freezing rain or sleet from rain or snow. The
[official flag table](https://github.com/NOAA-National-Severe-Storms-Laboratory/mrms-support/blob/main/GRIB2_TABLES/UserTable_MRMS_PrecipFlags.csv)
has only: no precip, warm stratiform rain, snow, convective rain, hail, cool stratiform rain and
two tropical mixes.

Decision: add class 3, **ice**, from HRRR. HRRR's CFRZR (freezing rain) and CICEP (ice pellets)
flags come from the full temperature profile.

1. The HRRR ingest always fetches the four type flags for `radar-hrrr`, and writes a numeric
   `precip-type` category grid per forecast hour. It no longer needs the standalone layer enabled.
2. The MRMS ingest reads the HRRR category grid valid nearest the frame time, at most 90 minutes
   away. Where MRMS has an echo and HRRR says freezing rain or ice pellets, the class is ice.
   Otherwise today's rain / snow / mix logic stands.
3. If no HRRR grid is close enough, the frame is rain / snow / mix as today. Radar never fails or
   waits for HRRR.

Legend: Rain, Snow, Mix, Ice. The inspector says "Ice: freezing rain or sleet, from HRRR model
guidance". It is model-assisted and the label says so.

Contract changes:

- Packed tiles: class 3 fits the existing byte (`3*32 + 31 = 127`). The packed metadata lists four
  classes.
- Palettes: add `types.ice` to the reflectivity entry in `classic`, `muted` and `vivid`.
- Old app builds: a client that knows three classes must still draw ice cells, in the mix colour.
  If the current format cannot guarantee that, bump the packed version and keep serving the old
  one. The ticket proves this with a fixture from a master build.
- Manifest: no new fields. `precip_type` stays a boolean.

### 3.2 The same colours on future frames

| Frames | Type source |
|---|---|
| Nowcast, 0–60 min | The newest observed type plane, held still. Type boundaries move slowly over one hour. |
| HRRR `radar-hrrr` | Same-hour CRAIN / CSNOW / CFRZR / CICEP. Snow and rain together is mix. Freezing rain or ice pellets is ice. |
| GFS `radar-gfs` tail | Same flags from GFS. The 0.25° file carries all four (checked in `gfs.t12z.pgrb2.0p25.f024.idx`). |

Extra download: four small records per forecast hour, read by byte range. No new schedule.

### 3.3 Product words

The storm-scale model is HRRR. Its dependable range is the 18 hours that every run reaches.

- One shared constant, `STORM_SCALE_HORIZON_S = 18 h`, in `web/src/shared`.
- Frames up to that lead: **Future radar**.
- Frames past it, from the HRRR extended runs and from GFS: **Precipitation forecast · model
  guidance**. Never "future radar".
- The timeline label, legend tag and inspector all read the same helper. A shared test pins the
  words on both sides of the line.

## 4. Snow and ice layers

One new workflow type, `IngestWinterWorkflow`, takes a product argument, like the global-model
ingest. It runs on the existing HRRR worker role. Each product gets its own schedule, so one slow
source cannot hold up another.

| Layer id | Shows | Source | Frames |
|---|---|---|---|
| `snowfall-forecast` | Snow still to fall, running total | NBM `ASNOW` 6-hour amounts, summed | 12 frames, +6 h to +72 h |
| `ice-forecast` | Ice build-up, running total | NBM `FICEAC` 6-hour amounts, summed | 12 frames, +6 h to +72 h |
| `snow-depth` | Snow on the ground | SNODAS snow depth, 1 km | 1 frame a day |
| `snowfall-24h` | Snow that fell in the last 24 h | NOHRSC snowfall analysis | Newest 24 h grid, twice a day |
| `snow-chance` | Chance of at least 1, 4 or 8 in of snow in 24 h | WPC Probabilistic Precipitation Portal | 3 thresholds × 24 h windows ending every 6 h |

`snow-chance` is one picker row with a 1 / 4 / 8 in chip. On the server it is three layer ids:
`snow-chance-1`, `snow-chance-4`, `snow-chance-8`.

Units on screen are inches. Ice is shown to 0.01 in. Every layer has a legend, an inspector value,
all three palettes, a tile retention entry and a numeric grid.

Why NBM for totals: every hourly NBM run reaches past 72 h. HRRR reaches 48 h only four times a
day. NBM `FICEAC` is ice that sticks. HRRR `FRZR` is the liquid that fell, which overstates it.

### Sources

| Source | Where | Format and grid | Cadence, latency | Terms |
|---|---|---|---|---|
| MRMS PrecipFlag | `https://noaa-mrms-pds.s3.amazonaws.com/CONUS/PrecipFlag_00.00/` | gzip GRIB2, 0.01° lat/lon, about 154 KB | 2 min, about 1.5 min late | [NODD open data](https://registry.opendata.aws/noaa-mrms-pds/) |
| HRRR | `https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.YYYYMMDD/conus/` | GRIB2 + `.idx`, Lambert 3 km | Hourly. 18 h, or 48 h at 00/06/12/18Z. About 55–107 min late | [NODD](https://registry.opendata.aws/noaa-hrrr-pds/): credit NOAA, no endorsement, say when modified |
| GFS | `https://noaa-gfs-bdp-pds.s3.amazonaws.com/gfs.YYYYMMDD/HH/atmos/` | GRIB2 + `.idx`, 0.25° | 4 runs a day | NOAA public domain |
| NBM | `https://noaa-nbm-grib2-pds.s3.amazonaws.com/blend.YYYYMMDD/HH/core/blend.tHHz.core.fNNN.co.grib2` | GRIB2 + `.idx`, Lambert 2.5 km, 2345×1597. One 6 h record is about 0.6 MB | Hourly runs, about 65–75 min late | [NODD](https://registry.opendata.aws/noaa-nbm/), same wording as HRRR |
| SNODAS | `https://noaadata.apps.nsidc.org/NOAA/G02158/masked/YYYY/MM_Mon/SNODAS_YYYYMMDD.tar` | Tar of gzip flat binary, int16 millimetres, 6935×3351 at 30 arc-seconds. About 8 MB | Daily, valid 06Z, posted about 14Z | [NSIDC G02158](https://nsidc.org/data/g02158/versions/1): citation required, doi:10.7265/N5TB14TC |
| NOHRSC snowfall | `https://www.nohrsc.noaa.gov/snowfall/data/YYYYMM/sfav2_CONUS_24h_YYYYMMDDHH_grid184.grb2` | GRIB2, Lambert 2.5 km, 2145×1377, `ASNOW` in metres. About 0.6 MB | 00Z and 12Z, about 5 h late | NWS public domain ([disclaimer](https://www.weather.gov/disclaimer)) |
| WPC PPP | `https://ftp-wpc.ncep.noaa.gov/prob_precip_portal/co/ppp_co_24hr_YYYYMMDDHH/ndfd_co_pw24ige{1p00,4p00,8p00}_YYYYMMDDHHfNNN.grib2`. Newest cycle in `co/latest_cycle.txt` | GRIB2, same 2.5 km grid. 0.35–1.7 MB each. `fNNN` is the end of the window | Hourly, done by about 10 min past | NWS public domain |

Notes the tickets must respect:

- The legacy WPC page and its `pwpf` files end on or about 2026-11-30
  ([SCN 26-96](https://www.weather.gov/media/notification/pdf_2026/scn26-96_PWPF_PQPF_Switching_to_PPP.pdf)).
  Build on the PPP files only. Use the `ftp-wpc` host: `ftp.wpc` only redirects to it.
- NBM core has no 24 h or 48 h snow total. Sum the 6 h records.
- **Not verified:** the stored scale of the PPP probability values (only headers were decoded).
  The `snow-chance` ticket proves 0–100 % with a real file before it draws anything.
- **Not verified:** that our Lambert decoder accepts the NBM and NDFD grids as-is. It handles
  template 3.30 on a spherical Earth. Each ticket checks a real file against a known point.
- NOHRSC also has hourly SNODAS GRIB2 in a `collaborators` folder. Its terms are not stated, so we
  do not use it.
- SNODAS is a new flat-binary reader: gunzip, read the text header, read int16.

## 5. Hyper-local snow: card and notice

### Snow at your spot

New route `GET /api/snow/{lat}/{lon}`. The server builds one small answer and caches it like the
forecast route.

| Field | From |
|---|---|
| Hourly snowfall, next 48 h | Self-hosted Open-Meteo `snowfall`. A live call on 2026-11-17 returned values in cm from the variables we already sync. |
| Snow on the ground, snow in the last 24 h | Point samples of `snow-depth` and `snowfall-24h` |
| Total next 24 / 48 / 72 h, ice total | Point samples of `snowfall-forecast` and `ice-forecast` |
| Chance of 1 / 4 / 8 in | Point samples of `snow-chance-*` |
| Start and end time | First and last hour with snowfall |

Each field is optional. A missing layer drops its line, not the card. Open-Meteo `snow_depth` is
null on our server because it is not synced. We use SNODAS and add no new sync variable.

The card shows: a headline ("3–5 in by Thursday 7 am"), hourly bars, on the ground now, fell in the
last 24 h, ice if any, and the chance of 4 in or more. The headline range is the NBM total and the
Open-Meteo total. When they agree within half an inch, show one number.

### Snow starts in ~N min

`GET /api/nowcast/{lat}/{lon}` gains one optional field per sample: the precip class at that point,
read from the typed nowcast frame. Old clients ignore it.

The shared planner in `web/src/shared/rainAlerts.ts` gains the type:

- Start class snow: "Snow starts in ~N min". Ice: "Freezing rain or sleet starts in ~N min".
  Mix: "Wintry mix starts in ~N min". Rain is unchanged.
- The snow start threshold is its own named constant. Snow reads lower on radar than rain for the
  same impact. The ticket sets the number from a recorded real snow event and keeps that fixture.
- Same opt-in, same lead choices, same dedupe. No new permission and no server push.

## 6. Everyday extras

### Nice Day and Bonfire index

One shared scoring module, `web/src/shared/niceDay.ts`. Input is one forecast hour: temperature,
feels-like, wind, rain chance, rain amount and humidity. Output is a 0–100 score and one of five
words: Great, Good, Fair, Poor, Do Not. Bonfire uses the same engine with evening weights: light
wind matters most, then dry, then cool but not cold. Sustained wind above 20 mph is always Do Not
for Bonfire.

The numbers are ours, not an official index, and the card says so. Thresholds live in one table
with a test vector file, `web/src/shared/testdata/niceDay.vectors.json`.

The map layer `nice-day` is scored on the server from the HRRR fields we already ingest. The Rust
scorer reads the same vector file in its tests, so the card and the map cannot drift.

### Countdowns and sunset

Pure client. A shared holiday table computes the US federal holidays by rule (fixed dates and
"nth weekday" dates), plus Easter, Halloween and the clock changes. No data source.

Sunset on a future date needs real sun maths, because the forecast only carries seven days. This
slice adds [`astronomy-engine`](https://github.com/cosinekitty/astronomy) (MIT, no dependencies,
pure JS) behind one wrapper, `web/src/shared/astro.ts`. The sky slices reuse the wrapper.

### Temp vs normal

| Piece | Decision |
|---|---|
| Normals | NCEI nClimGrid-Daily 1991–2020, `tmax` and `tmin` files, from `https://www.ncei.noaa.gov/data/oceans/archive/arc0207/0259962/1.1/data/0-data/`. 1/24° grid, lower 48 only, degrees C. Each file is 1.2 GB. The header says "no restrictions". |
| File format | NetCDF-4 (the first bytes are the HDF5 signature). The array is not compressed: 366 × 596 × 1385 × 4 bytes is 57,840 bytes less than the file. No C libraries in the image. Use a pure-Rust reader, or read the block by offset and prove it with known-point checks. |
| Bootstrap | A daily schedule `ensure-static-data` on the aux role. If the marker file is missing, it downloads, converts to int16 tenths of a degree, writes to a temp file, renames, then writes the marker. If the marker is present it does nothing. |
| Stored | `STATE_DIR/static/normals/{tmax,tmin}.i16`, about 0.6 GB each. Not under `GRID_DIR`, so the grid sweep cannot remove it. |
| Forecast side | GFS `TMAX` and `TMIN` 6-hour records, already in the 0.25° file. Daily high is the max over 06Z to 06Z. |
| Layer | `temp-anomaly`: forecast high minus normal high, days 1 to 7, one frame a day. Legend in °F above or below normal. |
| Point line | `GET /api/normals/{lat}/{lon}?date=` returns the normal high and low. The clients show "High 58°, 12° above normal". |

Compare a daily high with the high normal, and a low with the low normal. Never compare one hourly
reading with a daily mean. Day 60 in the file is Feb 29: skip it in other years. ECMWF data is
left out for now. GFS is enough to start, as the epic says.

### White Christmas chance

Source: NCEI daily station normals 1991–2020, column `DLY-SNWD-PCTALL-GE001WI` on Dec 25
([bulk tar](https://www.ncei.noaa.gov/data/normals-daily/1991-2020/archive/us-climate-normals_1991-2020_v1.0.1_daily_multivariate_by-station_c20230404.tar.gz),
303 MB, public domain). About 4,600 stations have a value.

The data never changes. A script builds one small JSON (station id, name, lat, lon, percent) and we
commit it with its provenance. The server serves it at `GET /api/climate/white-christmas` and
answers the nearest station for a point. The card says "62% of years had snow on the ground on
Dec 25 at Grand Rapids airport, 6 mi away". The map shows station dots. We do not interpolate.

The file documentation says the unit is tenths of a percent, but the values run 0.0 to 100.0. The
ticket checks three known cities before it trusts the scale.

## 7. Sky

### Look up tonight

| Piece | Decision |
|---|---|
| ISS orbit | CelesTrak `https://celestrak.org/NORAD/elements/gp.php?CATNR=25544&FORMAT=CSV`. Use the OMM format: the old TLE format is being retired. |
| Fetch rule | One scheduled fetch every 6 hours on the aux role, with a clear User-Agent. Write `STATE_DIR/iss.json` atomically. On any non-200, stop and keep the last good file until the next slot. No retries. Only the `.org` host. This follows the [usage policy](https://celestrak.org/usage-policy.php). |
| Passes | Server-side with the [`sgp4`](https://github.com/neuromorphicsystems/sgp4) crate (MIT, pure Rust). `GET /api/sky/tonight/{lat}/{lon}` returns passes for the next three nights. |
| Visible pass | Sun at least 6° below the horizon for the viewer, ISS in sunlight, ISS at least 10° high. These are common practice values, stated as ours. |
| Planets, moon, twilight | Client-side with the shared `astro.ts` wrapper. No data. |
| Clear or cloudy | The point forecast adds hourly `cloud_cover`. It is already synced and returned values in a live call. |
| Notice | A local notification about 10 minutes before a good pass, opt-in, built like the rain notice. |

Elements older than 14 days hide the ISS lines and say why. CelesTrak states no formal licence, so
we credit "Orbital elements: CelesTrak" and never re-serve the raw file.

### Star forecast map

Layer `stargazing`, scored 0–100 on the server for each night hour in the HRRR range.

| Input | Source | Slice |
|---|---|---|
| Cloud | HRRR total cloud cover, already ingested | star-map |
| Darkness | Sun at least 12° down, computed. Day pixels are transparent. | star-map |
| Moon | Phase and height, computed. A bright high moon lowers the score. | star-map |
| Transparency and seeing | ECCC RDPS astronomy fields, 0–5 scale, hourly to 84 h | star-seeing |

Sun and moon positions use short low-precision formulas in Rust. Tests compare them with
`astronomy-engine` values to within half a degree.

ECCC source: `https://dd.weather.gc.ca/today/model_rdps/10km/{HH}/{hhh}/{run}_MSC_RDPS_SeeingIndex_EAtm_RLatLon0.09_PT{hhh}H.grib2`
and `SkyTransparencyIndex`. Files are 0.5–1.1 MB. Terms:
[ECCC end-use licence](https://eccc-msc.github.io/open-data/licence/readme_en/), credit "Data
Source: Environment and Climate Change Canada". Build the file names from the run time. Never poll
the directory listing ([usage policy](https://eccc-msc.github.io/open-data/usage-policy/readme_en/)).

`star-seeing` is the risky slice. The files use a rotated lat/lon grid (template 3.1) and JPEG 2000
packing (5.40). Our decoder supports neither today. If a pure-Rust path is not possible, the slice
stops and reports. The star map still ships on cloud, darkness and moon.

Left out, with reasons:

- **Light pollution.** The well-known Falchi atlas forbids redistribution. The Lorenz atlas has no
  licence. VIIRS night lights are CC BY 4.0 but the download needs a free login. See decisions.
- **Aurora.** NOAA SWPC has a free feed. It is a good later slice, not part of this build.

## 8. Storage and cache cost

Measured on the cluster on 2026-11-17:

| Volume | Used | Size |
|---|---|---|
| `tiles-v2` | 9.0 GB | 64 Gi |
| `grids` | 9.6 GB | 20 Gi |
| `state` | 0.01 GB | 5 Gi |
| `openmeteo-data` | 15.1 GB | 60 Gi |

Estimates for the new work. Each layer ticket reports its measured bytes in the PR.

| Item | Volume | Estimate |
|---|---|---|
| About 230 new numeric frames at up to 3.2 MB (900×900 cap), with turnover | `grids` | about 1.5 GB |
| PNG pyramids for 10 new layers, three palettes | `tiles-v2` | under 3 GB |
| Ice class in typed radar | `tiles-v2` | none. Same bytes. |
| Normals | `state` | 1.2 GB |
| ISS elements, white Christmas JSON | `state`, image | under 1 MB |
| Open-Meteo | `openmeteo-data` | none. No new sync variable. |

`grids` goes from about 48% to about 55% full. No volume needs to grow. If a ticket measures more
than twice its estimate, it stops and says so before merging.

Retention: each new layer adds its own line to the fixed tile retention map in
`rust/crates/radar-aux/src/tile_cleanup.rs`. A layer missing from that map is never swept.
Forecast layers keep 12 h. `snow-depth` and `snowfall-24h` keep 48 h. `temp-anomaly` keeps 36 h.

API caches are small and byte-capped: `/api/snow` and `/api/sky/tonight` 16 MiB each, keyed on
coordinates rounded to 0.1°, 15 minute TTL.

Download cost is small. The largest is the one-time 2.4 GB normals fetch. NBM is about 15 MB an
hour, WPC about 15 MB an hour, everything else under 10 MB a day.

## 9. Temporal and cluster changes

New workflow types and schedules. All are additive. No existing workflow changes its command
order, so old runs replay cleanly.

| Schedule | Workflow | Every | Role |
|---|---|---|---|
| `ingest-nbm-snow` | `IngestWinterWorkflow` (NBM snow + ice) | 30 min | hrrr |
| `ingest-snow-depth` | `IngestWinterWorkflow` (SNODAS) | 1 h | hrrr |
| `ingest-snowfall-24h` | `IngestWinterWorkflow` (NOHRSC) | 1 h | hrrr |
| `ingest-snow-chance` | `IngestWinterWorkflow` (WPC) | 1 h | hrrr |
| `ingest-temp-anomaly` | existing `IngestGlobalModelWorkflow`, new product | 1 h | hrrr |
| `ingest-star-seeing` | `IngestSkyWorkflow` (ECCC seeing product) | 1 h | aux |
| `ingest-iss` | `IngestSkyWorkflow` | 6 h | aux |
| `ensure-static-data` | `EnsureStaticDataWorkflow` | 24 h | aux |

Each poll is a cheap "is there a new cycle" check. Work happens only when the source has new data.

Each slice follows [temporal-safe-deployments](../temporal-safe-deployments.md): add names to
`radar-types`, register on the role, seed the schedule, add a replay test, and never edit an
existing schedule's arguments in place. The typed-radar slices change activities only, not
workflow command order.

Cluster changes are a separate PR in `talos-argocd-proxmox`, off `main`, opened after the master PR
merges and the image is bumped:

- Add `nice-day` and `stargazing` to `HRRR_ENABLED_LAYERS` in
  `my-apps/personal-projects/radar-ng/temporal-workers/release-env-patch.yaml`.
- Nothing else. No new volume, worker pool or secret. Winter products are on by default in code.

Until that PR merges, the two HRRR-derived layers are absent and both clients hide their rows.

## 10. Build split

In dependency order, winter first. Three lanes run side by side, so at most three or four cards
are runnable at once. A slice also waits for the one above it in its lane, because they edit the
same files.

| # | Slice | Server | Web + app (same PR) | Needs |
|---|---|---|---|---|
| 1 | [`winter-ice-class`](https://github.com/mitchross/radar-ng/issues/316) | HRRR type grid; ice class in typed radar; palettes; packed class 3 | Four-ramp legend, inspector text, decoder tests | none |
| 2 | [`winter-future`](https://github.com/mitchross/radar-ng/issues/317) | Types on nowcast, HRRR and GFS frames | Future frames coloured; "Future radar" vs "Precipitation forecast" words | 1 |
| 3 | [`snow-forecast`](https://github.com/mitchross/radar-ng/issues/318) | `IngestWinterWorkflow`; NBM `snowfall-forecast` and `ice-forecast` | Winter picker group, legends, inspector | none |
| 4 | [`snow-ground`](https://github.com/mitchross/radar-ng/issues/319) | SNODAS `snow-depth`; NOHRSC `snowfall-24h` | Two Winter rows | 3 |
| 5 | [`snow-chance`](https://github.com/mitchross/radar-ng/issues/320) | WPC PPP `snow-chance-1/4/8` | One Winter row with a threshold chip | 4 |
| 6 | [`snow-spot`](https://github.com/mitchross/radar-ng/issues/321) | `GET /api/snow/{lat}/{lon}` | Snow at your spot card | 5 |
| 7 | [`snow-notice`](https://github.com/mitchross/radar-ng/issues/322) | Precip class in the point nowcast | "Snow starts in ~N min" on both, local notification words | 2 |
| 8 | [`nice-day`](https://github.com/mitchross/radar-ng/issues/323) | none | Shared scorer, Nice Day and Bonfire card | none |
| 9 | [`countdowns`](https://github.com/mitchross/radar-ng/issues/324) | none | `astro.ts` wrapper, holiday countdown with sunset | 8 |
| 10 | [`white-christmas`](https://github.com/mitchross/radar-ng/issues/325) | Station JSON and `GET /api/climate/white-christmas` | Seasonal card, station dots | 9 |
| 11 | [`sky-tonight`](https://github.com/mitchross/radar-ng/issues/326) | `ingest-iss`, `GET /api/sky/tonight`, hourly cloud cover in the point forecast | Look up tonight card, pass notice | 10 |
| 12 | [`nice-day-layer`](https://github.com/mitchross/radar-ng/issues/327) | `nice-day` HRRR layer from the shared vectors | Layer row, legend | 7, 8 |
| 13 | [`temp-normal`](https://github.com/mitchross/radar-ng/issues/328) | Normals bootstrap, `temp-anomaly`, `GET /api/normals` | Layer row and "above normal" line | 6 |
| 14 | [`star-map`](https://github.com/mitchross/radar-ng/issues/329) | `stargazing` layer: cloud, darkness, moon | Layer row, legend, best-window line on the card | 11, 12 |
| 15 | [`star-seeing`](https://github.com/mitchross/radar-ng/issues/330) | ECCC seeing and transparency, folded into the score | Legend detail and credit | 14 |
| 16 | integrate | All gates on the combined branch, then the master PR | Feature table in the PR | all |

Lanes: radar and HRRR render (1, 2, 7, 12, 14, 15), winter ingest (3, 4, 5, 6, 13), client cards
(8, 9, 10, 11).

Every slice runs the [AGENTS.md](../../../AGENTS.md) gates for what it touched: Rust format, clippy
and tests; web tests and build; app lint, types and tests; docs links. Slices with a new workflow
also run the worker replay and integration tests. A real source file, cut small, backs every new
decoder test. App delivery still needs a native rebuild.

Shared files are additive-only: the API route table, the schedule list, the worker registration,
the layer lists on both clients, the palettes and the retention map. Each PR merges the newest
`feat/weather-extras` first and keeps both sides of any conflict in those files.

## 11. Decisions for Mitch

None of these blocks the build. Each has a default that the tickets follow.

| Decision | Default |
|---|---|
| Where "future radar" ends | 18 hours. Past that the label is "Precipitation forecast · model guidance". Change one constant to move it. |
| Ice on observed radar comes from a model | Accept, with the label saying so. The alternative is no ice class on observed frames. |
| Light pollution on the star map | Leave out. To add it, download the VIIRS night lights file once with a free login and we ingest it as a static base. |
| Euro model for temp vs normal | Leave out. GFS first. |
| ISS pass push from the server | No. Local notifications only, like rain. |
| Nice Day thresholds | Ours, in one table. Tune after a week of use. |
| Aurora | Later slice. |

Credits to add to [third-party terms](../../reference/third-party-terms.md) as each slice lands:
NOAA/NWS (MRMS, HRRR, GFS, NBM, WPC, NOHRSC, NCEI), SNODAS via NSIDC with its DOI, Environment and
Climate Change Canada, CelesTrak, and the MIT notices for `astronomy-engine` and `sgp4`. Each
states "processed by radar-ng; not an official NWS product".
