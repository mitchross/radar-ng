# radar-grib

GRIB2 decoding and NOAA data access for the radar-ng Rust port. It replaces the Python
`pygrib` + `pyproj` + `boto`/`httpx` code in `backend/ingest_mrms`, `backend/ingest_hrrr` and
`backend/ingest_airquality`. Specs: `specs/spec-ingest-mrms-hrrr.md` (sections 4, 5, 12, 17),
`specs/spec-ingest-aux.md` (section 5). Defect decisions: `docs/PLAN.md`.

Status: all tests pass (35 unit, 8 golden). Nothing has been run against live NOAA data, because
NOAA is not reachable from the build machine. See "Must verify against real NOAA files" below.

## What it covers

- `decode`: GRIB2 message listing, selection and decoding to `radar_geo::Grid`. Grid templates 3.0
  (regular lat/lon) and 3.30 (Lambert conformal, spherical earth). Data templates 5.0, 5.2, 5.3
  (spatial differencing order 1 or 2) and 5.41 (PNG, 16-bit grayscale).
- `units`: kelvin to Fahrenheit, m/s to mph, mm to inches, wind speed, and the HRRR precip-type
  category grid. f32 operation order follows numpy.
- `mrms`: gunzip (multi-member accepted), south-up orientation, key timestamp parsing, the
  MRMS listing for the current UTC day, and a download that validates the key first.
- `idx`: `.idx` parsing and byte-range planning (`pick_ranges`). Ranges are not merged.
- `s3`: anonymous S3 ListObjectsV2 over HTTPS (pagination, prefix, continuation token, max-keys
  1000), whole-object GET, and ranged GET that requires 206.
- `hrrr`: file keys, run ids, the latest-run finder (HEAD f01, 12 candidate hours), the 13 HRRR
  variables with their `.idx` matchers and GRIB2 selectors, and `fetch_hour` (index, then ranges).
- `naqfc`: AQMv7 URL builders (AWS mirror, then NOMADS), latest-run finder, and whole-file download.
- `http`: shared reqwest client builder and the timeouts from the Python code.

## Public API summary

- `decode::{decode_message, decode_first_message, find_message, list_messages}`,
  `MessageSelector::{parameter, at_level, matches}`, `MessageInfo`, `DecodedMessage`,
  `SURFACE_*` constants, `linspace`, `nanmedian`.
- `mrms::{gunzip, decode_gz, ensure_south_up, extract_timestamp, format_valid_time,
  timestamp_hint, list_recent_keys, fetch_gz}`.
- `idx::{parse_idx, pick_ranges, IdxRecord, ByteRange}`.
- `s3::{S3Client, ListPage, parse_list_objects, MAX_KEYS}`; methods `list_page`, `list_all`,
  `get_object`, `get_range`, `head_status`.
- `hrrr::{HrrrVar, file_key, run_id, parse_run_id, horizon_for_run, latest_run_candidates,
  find_latest_run, fetch_hour, Fetched, BASE_URL}`.
- `naqfc::{AqmSources, AQM_BASE, AQM_NOMADS_BASE, AQM_CYCLES, split_run_id, find_latest_run,
  download_product}`.
- `units::*`, `http::{client, client_builder, USER_AGENT}` and the timeout constants.
- `error::{GribError, Result}`. `GribError::is_not_found()` is true for HTTP 404 only.

Callers pass `now` explicitly to the time-dependent functions, so tests do not depend on the clock.

## Tests

Run from `rust`: `cargo test -p radar-grib`.

- Unit tests (35, all pass): decode helpers, selectors, packing, mrms keys and gunzip, idx parsing
  and planning, S3 XML parsing (hand-written ListBucketResult documents, entity unescaping,
  truncation and token handling, malformed input), 206 Content-Range and length checks, HRRR keys
  and selectors, NAQFC URLs and run ids.
- Golden tests (8, all pass) in `tests/golden.rs`. Fixtures in `testdata/` were produced by
  `testdata/gen_fixtures.py` (eccodes 2.49.0) and `testdata/gen_projection.py` (pyproj). Checks:
  - MRMS regular lat/lon: f32 values bitwise equal to eccodes; lat/lon axes within 1e-9.
  - MRMS: NaN count equals the eccodes missing count; the .gz path decodes identically.
  - HRRR-like Lambert messages (5.0, 5.2, 5.3 order 2): f32 values bitwise equal; codes and
    levels match the eccodes names; lat/lon within 1e-9 of eccodes.
  - Projected axes from the spec 5.3 procedure within 1e-3 m of the pyproj reference.
  - `radar_geo` LCC forward within 1e-3 m and inverse within 1e-9 degrees of pyproj.

The golden fixtures are generated GRIB2 files, not NOAA downloads. They cover the packing
templates and the codes, not the byte-level details of real HRRR or MRMS files.

## Deviations from the Python and spec

- User-Agent is `radar-ng-ingest/0.1 (radar-grib; rust)`. httpx sent its own default. NOAA does
  not require a specific value.
- No in-process retries. Temporal's activity retry policy owns retries, as in the Python.
- PLAN D4: the HRRR `.idx` falls back to the whole object only on HTTP 404. Python falls back on
  any HTTP error, which can download about 130 MB on a 5xx or a timeout. Other index errors fail.
- PLAN D5: ranged GET requires 206. A Content-Range start that differs from the request, or a body
  whose length differs from the range, is an error. Without this, a server that ignores Range
  would be written once per range.
- PLAN D6: the S3 prefix and continuation token are percent-encoded by reqwest's query encoder.
  The Python interpolates them raw into the URL.
- PLAN D21: `fetch_gz` parses the MRMS key timestamp before downloading. Python parses it after.
- Python's `_list_keys_sync` filter is kept (`.grib2.gz` only). `list_recent_keys` does not
  validate timestamps; `fetch_gz` does.
- MRMS and AQM downloads are returned in memory. Python streams the AQM file to a temp file.
- `decode_message` decodes the first message. Use `find_message` with a `MessageSelector` for
  name-based selection. Python selects by eccodes name (`shortName`, `typeOfLevel`, `level`).
- HRRR selectors use GRIB2 codes (discipline, category, number, first-surface type, level), not
  name strings. Name strings differ between eccodes versions ("TMP" and "2 metre temperature").
  The code list is in `hrrr::HrrrVar::selector`.
- Scale-factor octet 0xFF is read as scale 0, as eccodes does. The `grib` crate reads it as -127,
  which gives a level of 2e127 for 2 m messages. The crate's own decoder does not use that value.
- Scaling uses f64: `(R + X*2^E) * 10^-D`, cast to f32 once. This matches eccodes bitwise on the
  fixtures. The `grib` crate computes in f32 and differed by 1 ulp on 436 of 2450 MRMS values.
- Templates 5.40, 5.42 and 5.200 use the `grib` crate's f32 path. That path is not bit-exact with
  eccodes and is not covered by golden tests.
- PNG (5.41): 16-bit grayscale is tested. 8-bit and other colour types are rejected as
  Unsupported.
- Lambert: spherical earth only. Ellipsoidal shapes are rejected as Unsupported.
- Rejected as Unsupported: regular_ll with a basic angle, unusual scan modes, group splitting
  (section 5, octet 22), missing-value management (in-line 1 or 2), predefined bitmaps, and any
  template not listed above.
- Lambert lat/lon come from the `grib` crate's `latlons_unchecked`. Longitudes are normalised to
  -180..180. Projected axes use the spec 5.3 procedure, with the Python 1-D fallback.
- MRMS south-up flip is `ensure_south_up`, applied by the caller, as in the Python.

## Deferred

- NAQFC: the Python `_extract_native_projection` path for the grid-227 curvilinear layout is not
  ported. The crate decodes the message through the same template path as HRRR. The result for
  the real file is unverified.
- NAQFC chunked rendering (`aqm_render_chunk` with `start_msg`). `download_product` returns the
  whole file, and per-message-range selection is left to the caller.
- Streaming downloads to disk. The caller owns temp directories and ProcessedSet state files.
- Cleanup of superseded runs (`aqm_cleanup`, HRRR versioned runs). Not in this crate.
- Reading `.grib2` files from disk is left to the caller. The API takes bytes.
- Ellipsoidal Lambert and 8-bit PNG.

## Must verify against real NOAA files

Run these on a machine with NOAA access. Each item has a concrete check. `rust/scripts/preflight.sh`
(step 4, `--only noaa`) automates items 1 to 10 and 12 against the production Python code; items 11
(beyond the pagination of one day's MRMS listing), 13 and 14 stay manual.

1. Missing objects in anonymous S3 return 403, not 404, when the caller has no ListBucket
   permission. The HRRR "pending" path and the `.idx` fallback depend on 404 (`hrrr::fetch_hour`,
   `fetch_full_file`, `naqfc::download_product`). If NOAA returns 403, pending hours error out
   instead of waiting, and the `.idx` fallback never runs. Check with `head_status` on a
   nonexistent key in `noaa-hrrr-bdp-pds` and in `noaa-nws-naqfc-pds`. Decide the mapping from the
   result.
2. HRRR `.idx` line format: fetch `hrrr.YYYYMMDD/conus/hrrr.tHHz.wrfsfcf01.grib2.idx` and confirm
   the fields `num:offset:d=...:VAR:level:fcst:`. Confirm that `pick_ranges` with
   `HrrrVar::idx_matcher` picks exactly the records that the Python `IDX_MATCHERS` pick, for all 13
   variables. Each of REFC, TCDC, APCP, CAPE and the categorical fields should match one record in
   f01. Check for duplicates in later hours, such as APCP accumulations.
3. Code-based selectors versus Python name selectors: on a real wrfsfcf01 file, run
   `list_messages` and compare with pygrib `shortName`, `typeOfLevel` and `level` for the
   matching messages. Confirm that `HrrrVar::selector` picks the same message for all 13 variables.
   Watch REFC (0.16.196, local table) and the categorical fields (0.1.192..195).
4. Decoded values against pygrib on a real HRRR subset: f32 bitwise for the 13 variables. The
   fixtures are synthetic-style messages from eccodes, so real packing (5.3 with order 2, possibly
   with bitmaps and group splitting) may hit the Unsupported paths.
5. Decoded values against pygrib on one real MRMS `.grib2.gz`: f32 bitwise, lat/lon within 1e-9,
   and the south-up flip matches Python.
6. Lambert lat/lon within 1e-9 of eccodes on a real HRRR message. Only the fixture has been checked.
7. Real messages do not hit `packing.rs` Unsupported paths (missing-value management, group
   splitting, predefined bitmap). If one does, implement it or document it.
8. Negative E and D scale factors decode the same as eccodes on real data.
9. NAQFC grid 227: the template (3.0 or 3.30) and the Python's projection handling. Compare the
   Rust grid with `_extract_native_projection` output for the same file.
10. NAQFC latest-run HEAD answers: confirm 200 on the AWS mirror and on NOMADS, and whether either
    redirects (redirects are not followed, so a 3xx counts as a miss). Python behaves the same way.
11. S3 listing: confirm that `prefix` and `continuation-token` values containing `/`, `=` and `+`
    encode the same way the Python request does, and that NOAA accepts the continuation token.
12. Ranged GET: confirm NOAA returns 206 with a matching Content-Range for the `.idx`-derived
    ranges, including the open-ended last range (an HRRR message at the end of the file).
13. MRMS: at 00:00 UTC, confirm the yesterday fallback in `list_recent_keys` returns the expected
    keys in listing order.
14. Timing: the 15 s, 120 s and 300 s timeouts are enough for the real files on the deployment
    network.

## Notes for the integrator

- All error paths return `GribError`. Map `is_not_found()` to "pending" only where the Python does.
- `hrrr::fetch_hour` returns bytes. Writing them to a temp file and calling `decode` is the caller's job.
- `S3Client::new(http::client()?, hrrr::BASE_URL)` is the production setup. Tests can point it at a
  local server.
