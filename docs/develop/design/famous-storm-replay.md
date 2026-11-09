# Famous-storm replay

**Proposed, not approved.** Design for [#262](https://github.com/mitchross/radar-ng/issues/262),
part of [#264](https://github.com/mitchross/radar-ng/issues/264). This note adds no feature.
Archive checks below ran on 2026-11-08. Approve the design before building.

## Recommendation

Ship Rolling Fork and Greenfield first. Precompute bounded regional cases on the server.
Reuse the MRMS decoder, tiler, manifest format and both clients' loop engines.
Put a public **Famous storms** picker beside the live timeline on web and the app's Radar tab.
Always show **Historical replay**, the full date and **Return to live**.
Never mix a replay with today's radar, warnings or forecasts. Clients call only our server.
This follows the [product principles](../../reference/north-star.md) and
[web + app parity rule](../../../AGENTS.md#web--app-parity).

## 1. Archive coverage

The [IEM archive guide](https://mesonet.agron.iastate.edu/archive/mrms.php) dates AWS MRMS
coverage to October 2020. Anonymous S3 ListObjectsV2 requests confirm **2020-10-14** as the
first available date for every current product below. These are first retained objects,
not proof of continuous coverage since then. The [AWS registry](https://registry.opendata.aws/noaa-mrms-pds/)
warns that the October 14 v11 → v12 change altered products and science.

Prefixes below sit under `CONUS/`. Product names come from
[ingest configuration](../../../rust/crates/radar-ingest/src/config.rs),
[composite schedule](../../../rust/crates/radar-types/src/schedules.rs) and
[single-field MRMS products](../../../rust/crates/radar-ingest/src/mrms_qpe.rs).
Use `MergedReflectivityComposite`, not the similarly named `MergedReflectivityQCComposite`,
to match the existing composite layer.

| Layer / exact product suffix | First object UTC on 2020-10-14 (listing source) | Rolling Fork objects | Greenfield objects |
|---|---|---:|---:|
| `radar`: `MergedBaseReflectivityQC_00.50` | [21:15:49](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FMergedBaseReflectivityQC_00.50%2F&max-keys=1) | 120 | 90 |
| `radar-composite`: `MergedReflectivityComposite_00.50` | [21:16:37](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FMergedReflectivityComposite_00.50%2F&max-keys=1) | 120 | 90 |
| `precip-24h`: `MultiSensor_QPE_24H_Pass1_00.00` | [21:00:00](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FMultiSensor_QPE_24H_Pass1_00.00%2F&max-keys=1) | 4 | 3 |
| `hail-size`: `MESH_Max_60min_00.50` | [00:00:25](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FMESH_Max_60min_00.50%2F&max-keys=1) | 120 | 90 |
| `rotation-track`: `RotationTrack60min_00.50` | [00:00:23](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FRotationTrack60min_00.50%2F&max-keys=1) | 120 | 90 |
| `echo-top`: `EchoTop_18_00.50` | [00:00:25](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FEchoTop_18_00.50%2F&max-keys=1) | 119 | 90 |
| Reflectivity companion: `PrecipFlag_00.00` | [00:00:00](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FPrecipFlag_00.00%2F&max-keys=1) | 120 | 90 |

Counts are actual listed GRIB objects in the half-open case windows in section 5, not rendered
frames or a completeness guarantee. Full daily queries returned `IsTruncated=false`.
For example, base reflectivity lists include
[`20230324-230015`](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FMergedBaseReflectivityQC_00.50%2F20230324&max-keys=1000),
[`20230325-000010`](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FMergedBaseReflectivityQC_00.50%2F20230325&max-keys=1000) and
[`20240521-190017`](https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS%2FMergedBaseReflectivityQC_00.50%2F20240521&max-keys=1000).
All use `MRMS_<product>_<YYYYMMDD-HHMMSS>.grib2.gz`.
Reproduce every product/date listing without credentials:

```bash
for product in MergedBaseReflectivityQC_00.50 MergedReflectivityComposite_00.50 \
  MultiSensor_QPE_24H_Pass1_00.00 MESH_Max_60min_00.50 \
  RotationTrack60min_00.50 EchoTop_18_00.50 PrecipFlag_00.00; do
  for day in 20230324 20230325 20240521; do
    aws s3 ls --no-sign-request "s3://noaa-mrms-pds/CONUS/$product/$day/"
  done
done
```

No archive GRIB was decoded or rendered for this note. Import must check gaps and values,
not assume an object is usable. Preserve real timestamps: some products arrive seconds apart,
and two-minute nominal cadence does not mean exact two-minute spacing.
Use base reflectivity as the master playhead; choose each overlay's latest source at or before
that time. Cap fast-layer staleness at five minutes and show missing data beyond that.
Show hourly QPE's own valid time; never interpolate it into two-minute observations.
QPE is a rolling 24-hour accumulation, not rainfall since this case began. Its gauge correction
also arrives later than valid time: label it retrospective, not what an observer knew then.

### Older archives are not a drop-in fallback

IEM documents selected [MTArchive](https://mtarchive.geol.iastate.edu/) products back to October
2014. A checked October 15 path returned 404; the
[November 1, 2014 listing](https://mtarchive.geol.iastate.edu/2014/11/01/mrms/ncep/)
contains PrecipRate and RadarOnly QPE, and
[January 1, 2015](https://mtarchive.geol.iastate.edu/2015/01/01/mrms/ncep/) also lists SeamlessHSR.
These do not establish older coverage of our exact six rendered products.

The checked [March 24, 2023](https://mtarchive.geol.iastate.edu/2023/03/24/mrms/ncep/) and
[May 21, 2024](https://mtarchive.geol.iastate.edu/2024/05/21/mrms/ncep/) raw indexes list selected
products including MESH, MESH_Max_1440min, RotationTrack1440min, SeamlessHSR and Pass2 QPE,
not our complete AWS set. Real files include
[MESH in 2023](https://mtarchive.geol.iastate.edu/2023/03/24/mrms/ncep/MESH/MESH_00.50_20230324-000000.grib2.gz)
and [MESH in 2024](https://mtarchive.geol.iastate.edu/2024/05/21/mrms/ncep/MESH/MESH_00.50_20240521-000000.grib2.gz).
Instantaneous MESH is not MESH_Max_60min; a 1440-minute track is not a 60-minute track.
Do not silently substitute them. IEM's hourly ZIP service is now only a rolling 30-day cache,
per its archive guide. MYRORSS reanalysis is another older source, but needs a separate design.
Use AWS only for the first release; defer pre-2020 cases.

### Units

Reuse `rotation_value` in [mrms_qpe.rs](../../../rust/crates/radar-ingest/src/mrms_qpe.rs).
MRMS RotationTrack is **0.001 s^-1**, so divide by 1000 to render s^-1; negative missing values
become NaN. Reuse mm → inches for MESH/QPE and km → kft for EchoTop.
The [NSSL GRIB table](https://www.nssl.noaa.gov/projects/mrms/operational/tables.php) is the
upstream units reference. A rotation track is a radar-derived history, not a confirmed tornado path.

## 2. Warnings in force

Use IEM's versioned [SBW interval API](https://mesonet.agron.iastate.edu/api/1/docs#/vtec/service_vtec_sbw_interval__fmt__get).
The [OpenAPI schema](https://mesonet.agron.iastate.edu/api/1/openapi.json) names the parameters
`begints` and `endts`; some prose still calls them `sdate` and `edate`.
Set `only_new=false&include_can=true`, not the issuance-only default.

| Verified query (one office, tornado warnings only) | Polygon versions returned | Example |
|---|---:|---|
| [Rolling Fork window, JAN](https://mesonet.agron.iastate.edu/api/1/vtec/sbw_interval.geojson?begints=2023-03-24T23:00:00Z&endts=2023-03-25T03:00:00Z&wfo=JAN&ph=TO&only_new=false&include_can=true) | 44 | JAN TO.W 0028 issued 00:53Z, expires 02:00Z March 25 |
| [Greenfield window, DMX](https://mesonet.agron.iastate.edu/api/1/vtec/sbw_interval.geojson?begints=2024-05-21T19:00:00Z&endts=2024-05-21T22:00:00Z&wfo=DMX&ph=TO&only_new=false&include_can=true) | 39 | DMX TO.W 0036 issued 19:55Z, expires 20:45Z May 21 |

These counts are neither tornado counts nor all warnings in the regional viewport.
Fetch all relevant offices and TO.W/SV.W/FF.W polygons intersecting each case region at import.
Seed warnings already active at case start using the legacy
[active-at-time service](https://mesonet.agron.iastate.edu/geojson/sbw.py?help), then fetch their
full update histories as well. An issuance-window query alone can miss them.
Actual snapshot checks returned 12 all-phenomena Mississippi features at
[Rolling Fork 01:00Z](https://mesonet.agron.iastate.edu/geojson/sbw.geojson?ts=2023-03-25T01:00:00Z&states=MS)
and 19 Iowa features at
[Greenfield 20:40Z](https://mesonet.agron.iastate.edu/geojson/sbw.geojson?ts=2024-05-21T20:40:00Z&states=IA).

Store event identity `(year, wfo, phenomena, significance, eventid)` and version identity
`(product_id, utc_polygon_begin)`. At frame time, select
`utc_polygon_begin <= frame < utc_polygon_end`, not just event issue/expire.
For JAN 0028 at 01:00Z, the correct CON version is 00:59–01:04Z.
For DMX 0036 at 20:40Z it is 20:27–20:45Z; DMX 0041 is also active.
The [original Rolling Fork bulletin](https://mesonet.agron.iastate.edu/api/1/nwstext/202303250053-KJAN-WFUS54-TORJAN)
and [Greenfield update](https://mesonet.agron.iastate.edu/api/1/nwstext/202405212027-KDMX-WWUS53-SVSDMX)
confirm the places and times.

Keep CAN history but do not draw cancellation as an active warning. EXP can precede actual
expiry: JAN 0028 has an EXP version at 01:55Z valid until 02:00Z.
Ignore zero-duration versions and resolve corrected products deterministically.
Do not use event-lifetime `max_is_emergency` or `max_is_pds` to style earlier frames;
cache contemporaneous bulletin text for these labels. Neither sampled interval contained CAN,
so cancellation still needs a separate fixture. Cache all histories once on our server,
not an IEM request per frame or per visitor.

## 3. Storage and rendering

**Precompute, not on-demand.** Request-time GRIB decoding would make a public case slow,
repeat expensive work and compete with live ingestion. Use an operator-started bounded import;
no public endpoint starts rendering.

Reuse [radar-tiler](../../../rust/crates/radar-tiler/src/render.rs) and its atomic publication.
Use existing zooms 4, 5, 6, 7 from [MRMS ingest](../../../rust/crates/radar-ingest/src/mrms.rs).
Fix the output XYZ set to tiles intersecting the case bounds. Render complete boundary tiles,
not rectangles clipped midway through a tile. Crop decoded input only to the union of those tile
footprints plus the source cells required by bilinear and overview/max-pool sampling; never crop
just to the case rectangle. Add an optional output-tile allowlist to the existing tiler so expanded
input bounds cannot emit extra halo tiles. This bounded selection and importer crop are new work,
not existing tiler options. The tile budget below counts the fixed output set, not the expanded
input grid. Keep native resolution, use existing overzoom above 7, constrain the replay viewport
to the case bounds, and compare boundary tiles with an uncropped render to prove seam parity.
Run one frame at a time from a plain thread: nested parallel calls can deadlock the tiler's
publication lock, as documented in [rust-dev](../../../.claude/commands/rust-dev.md).

First release: one `classic` PNG pyramid per layer, no packed buckets, retained point grids or
extra palette variants. Both clients already have PNG paths; replay must explicitly select them
and disable unavailable inspector readouts. PrecipFlag colors reflectivity, not a seventh pyramid.
Retain source keys, checksums, renderer version and warning provenance for reproducibility.
Remove temporary full-CONUS GRIBs after verified publication, not through the live retention sweep.

Planning estimate, **not measured PNG output**:

`bytes = (fast frames × 5 layers + hourly QPE frames) × tiles across z4..7 × mean PNG bytes`

| Case bounds `[west,south,east,north]` | Fast frames budget | QPE | Tiles at z4 / z5 / z6 / z7 | PNG files budget | At 20–80 KiB/PNG |
|---|---:|---:|---|---:|---:|
| Rolling Fork `[-92,31,-88,35]` | 120 | 4 | 2 / 4 / 4 / 6 | 9,664 | 0.18–0.74 GiB |
| Greenfield `[-97,40,-92,44]` | 90 | 3 | 2 / 2 / 4 / 9 | 7,701 | 0.15–0.59 GiB |

Tile counts use the existing [Web Mercator geometry](../../../rust/crates/radar-tiler/src/geometry.rs),
counting every intersecting XYZ tile, even if transparent. Five fast layers are base, composite,
hail, rotation and echo top. Rounded frame budgets cover the listed echo-top shortfall, not future
unbounded imports. Compression assumptions need a representative render before approval to import.
Both cases total about 0.33–1.32 GiB of PNG payload; filesystem blocks, provenance, warnings,
source staging and atomic replacement need additional space.

The [tiles PVC default](../../../deploy/kubernetes/base/pvcs.yaml) is **100 GiB**;
a read-only live `kubectl get pvc -n radar-ng tiles` check also reported 100Gi.
That is capacity, not free space. Recommend a 10 GiB combined replay quota including staging,
with at least 20 GiB free left for live data. Measure actual allocated bytes and inodes before import.
If the pilot exceeds quota, reduce scope or request a separate volume; never evict live frames.
A full extra palette roughly adds another corresponding pyramid; it is outside this estimate.

Store cases beneath an isolated `replays/<case>/<revision>/` root on the tiles PVC, with a separate
manifest per case. Do not insert old frames into the live manifest, trigger nowcast, or let live
cleanup sweep this root. Publish a revision only after every advertised tile and warning file is
ready. An interrupted import leaves the prior revision readable. Ordinary viewers cannot delete cases.

## 4. Server and timeline contract

These are **proposed routes**, not existing endpoints:

| Route | Response / behavior |
|---|---|
| `GET /api/replays` | Ready catalogue: `schema_version`, `cases[]` with `id`, `revision`, `title`, `summary`, `start`, `end`, `timezone`, `bounds`, `layers`, `manifest_url`, `warnings_url`, `sources` |
| `GET /api/replays/{id}/{revision}/manifest.json` | Existing manifest v2 shape, case-only frames, per-layer units/palettes and namespaced tile template |
| `GET /api/replays/{id}/{revision}/warnings.geojson` | Cached polygon version collection, normalized labels and validity fields; clients filter against playhead time |
| `GET /replays/{id}/{revision}/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png` | Immutable case PNGs; unknown case/revision is 404, never a live fallback |

Reuse manifest types in [radar-core](../../../rust/crates/radar-core/src/manifest.rs) and the
ETag/last-good behavior of [radar-api's manifest handler](../../../rust/crates/radar-api/src/manifest.rs).
The current route is registered in [edge.rs](../../../rust/crates/radar-api/src/edge.rs), not
`lib.rs`. Keep `/api/manifest.json` unchanged. Validate allowlisted case IDs and paths, keep import
writes private, add the new GET paths to any API edge/gateway allowlist, and cap response size.
A revision pins manifest, warning and tile URLs together; clients use the catalogue's exact URLs,
never a mutable “latest” alias. Keep the previous revision until readers can finish. Revision and
source context belong in every client cache key; catalogue/manifest errors remain explicit errors.

A manifest swap alone is insufficient. [Web frames](../../../web/src/frames.ts) merge live radar
with nowcast/HRRR/GFS and thin the long loop; the app's
[useManifest](../../../frontend/src/hooks/useManifest.ts) uses wall-clock now and
[playbackSequence](../../../frontend/src/lib/playbackSequence.ts) also thins old frames.
The app's [tile URL builder](../../../frontend/src/lib/tileUrl.ts) currently hardcodes `/tiles`.
Add an explicit `live | replay(caseId, revision)` source context on both clients:

1. Cancel stale source requests and clear displayed/prefetched frames when switching context.
2. Load the case manifest into the existing frame model, but bypass forecast merging and live
   age filters. Start at the first base frame; loop all its indices without 15-minute thinning.
3. Reuse play/pause, stepping, scrubbing and readiness-gated prefetch. Keep a bounded lookahead,
   not all case frames in memory. Preserve UTC playhead when changing layers; select the latest
   eligible layer frame, or show a gap. Support composite in both clients, not only the web.
4. Use the case's tile base in every network/disk cache key. Label the axis with absolute historical
   date/time and timezone, never “Now” or today's freshness status.
5. Filter cached warning versions at that playhead time using the same fixtures on web and app.
   Adapt existing warning colors, tags and popups; don't poll live map warnings during replay.
6. Hide unsupported live overlays and inspector endpoints (storms, satellite, lightning, SPC,
   tropical, wind and point forecasts). Replay warnings must never feed notification code.
   Keep real safety notifications separate and clearly live; returning to live restores current
   preferences and fetches a fresh manifest before displaying it.

[Web main](../../../web/src/main.ts) already coordinates playhead and prefetch;
[web overlays](../../../web/src/overlays.ts) and the app's
[WarningsOverlay](../../../frontend/src/components/map/WarningsOverlay.tsx) currently load live
alerts, so both need explicit source routing. Neither client may fetch AWS or IEM directly.

## 5. First cases

Ship **two**, not four. Both have checked MRMS objects and warning history, exercise intense
convection, and keep the first storage/runtime budget small. Windows include context before and
after the surveyed tornado, not just city impact. Store UTC half-open windows; display America/Chicago.

| Case ID / public title | Replay UTC window | Why / primary event source |
|---|---|---|
| `rolling-fork-2023` / Rolling Fork, Mississippi — March 24, 2023 | March 24 23:00Z → March 25 03:00Z | EF4, nocturnal storm and UTC-midnight test. NWS Jackson's [2023 survey KML](https://www.weather.gov/media/jan/events/2023.kml), linked from its [event archive](https://www.weather.gov/jan/2023tornadoinfo), records the Rolling Fork tornado 00:57–02:08Z March 25. |
| `greenfield-2024` / Greenfield, Iowa — May 21, 2024 | May 21 19:00Z → 22:00Z | EF4 daytime case with overlapping warning versions. [NWS Des Moines](https://www.weather.gov/dmx/2024-May-21-Iowa-TornadoesWindsFlooding) gives 2:57–3:43 pm CDT, or 19:57–20:43Z. |

Use respectful, factual descriptions and source attribution. Defer Joplin 2011 and Moore 2013:
they predate the verified AWS product set. Do not promise those cases from an unrelated reanalysis.

## 6. Parity and ticket-sized build split

[AGENTS.md](../../../AGENTS.md#web--app-parity) requires every user-facing feature on both clients
in the same change. Backend groundwork may precede it; no web-only launch. This table is a proposed
split for later tickets, not authorization to implement or a new set of cards.

| Ticket | Server piece | Web piece | App piece / acceptance |
|---|---|---|---|
| 1. Bounded case importer | Validated case definitions; real-object/gap checks; unit conversions; crop + existing tiler; quota; resumable atomic publication; isolated retention | None yet | None yet. Import a small pilot; measure disk/RAM/render time and prove no live manifest/nowcast/cleanup side effects. |
| 2. Historical warnings | Cache seed snapshot and update history for regional TO/SV/FF warnings; bulletin provenance; normalize version validity | Shared fixture expectations | Same expectations. Test NEW/CON/CAN/EXP/correction, existing-at-start, exact boundaries and UTC midnight; no future PDS/emergency leakage. |
| 3. Read-only case API | Catalogue, revisioned manifest/tiles/warnings, ETags, allowlists, path validation and explicit missing-case errors | Contract fixtures | Identical fixtures; prove clients need only our server. Depends on 1–2. |
| 4. Replay experience, both clients in one PR | Finalize contract without changing live endpoints | Picker by timeline, source context, full-case loop, timestamps, warning popups, Return to live | Same on Radar tab; hook/store/cache isolation; composite and PNG path parity. Depends on 3. No historical notifications or unrelated live overlays. |
| 5. Two-case acceptance and publication | Build both complete cases, record coverage/gaps, checksums and allocated bytes; document operator import/rebuild | Browser exercise: load, scrub, change layer, loop, slow network, missing data, return live | Same on real iOS/Android builds; memory/prefetch check, notification isolation and shared frame/warning boundary fixtures. Publish only after both pass. |

Run Rust gates for server code, web tests/build, app lint/types/tests and docs link checks as
specified in AGENTS.md. App delivery needs a native rebuild; a web build is not phone verification.
No code, imports, cluster changes or native builds occur in this design PR.

## 7. Decisions for Mitch

| Decision / risk | Recommended option |
|---|---|
| Approve scope and case windows? | Two cases above, six layers, regional bounds, historical TO/SV/FF warnings. Add cases only after measured acceptance. |
| Space versus palette/zoom flexibility? | One classic PNG set, z4–7, 10 GiB total replay quota and 20 GiB live headroom. Measure the pilot first; defer packed data, inspector and extra palettes. |
| Historical fidelity versus invented continuity? | Preserve source timestamps and visible gaps. Label hourly retrospective QPE and 60-minute tracks honestly; never interpolate or substitute different products. |
| Warn about historical hazards without confusing live safety? | Persistent replay banner/date, explicit return action, no replay-triggered notifications, and no unrelated live map overlays. |
| Archive retention, cancellations and corrected warnings? | Pin source provenance/fixtures during import; require cancellation/correction tests before release. AWS/IEM availability checks are not an SLA or a full quality audit. |

**Approval gate:** agree this note on #262 before any implementation. Merging the design does not
close the feature issue or satisfy its web/app replay acceptance item.
