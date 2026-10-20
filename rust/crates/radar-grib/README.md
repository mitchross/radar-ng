# radar-grib

GRIB2 decoding and NOAA data access for radar-ng: MRMS, HRRR and NAQFC listing, download, `.idx`
range planning and decoding, on the pure-Rust `grib` crate plus anonymous S3 over HTTPS. Specs:
`docs/develop/specs/spec-ingest-mrms-hrrr.md` (sections 4, 5, 12, 17) and
`docs/develop/specs/spec-ingest-aux.md` (section 5).

Tests: 35 unit, 8 golden. CI's `live-noaa` job (`rust/scripts/live-noaa-check.sh`) decodes the newest
live MRMS and HRRR files on every PR.

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
- `http`: shared reqwest client builder and the request timeouts.

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
  `testdata/gen_fixtures.py` (eccodes 2.49.0) and `testdata/gen_projection.py` (pyproj); regenerating
  them needs those Python packages. Checks:
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

## Notes for the integrator

- All error paths return `GribError`. Map `is_not_found()` to "pending" only where the Python does.
- `hrrr::fetch_hour` returns bytes. Writing them to a temp file and calling `decode` is the caller's job.
- `S3Client::new(http::client()?, hrrr::BASE_URL)` is the production setup. Tests can point it at a
  local server.

## Verified against real NOAA files (2026-10-14)

Before the Python backend was removed, a one-time CI comparison against the newest live files and the
production Python decoder (pygrib/eccodes) found:

- MRMS `MergedBaseReflectivityQC_00.50`: values bitwise equal to pygrib, axes within 1e-9, and the
  rendered pyramid identical to Python's indexed renderer (`radar parity --exact`, 675/675 tiles). Fixed
  on the way: template 3.0 basic angle 1 / 10^6 subdivisions (MRMS) is now honored.
- HRRR `wrfsfcf01`: all 13 radar-ng variables selected, `.idx` picks equal to Python's, values bitwise,
  Lambert lat/lon within 1e-9. 168 of 170 messages are bitwise equal; the two that differ are cloud
  ceiling and cloud top height (0.3.5, complex packing with a bitmap and E<0), which radar-ng doesn't
  read. Categorical precipitation types are matched by both the NCEP-local (0.1.192-195) and WMO
  (0.1.33-36) numbers.
- NAQFC grid 227 decodes (template 3.30, data 5.3). The AWS mirror answered 404 for the newest run and
  NOMADS 200, so the NOMADS fallback is exercised.
- Missing objects answer 404 on all three buckets, so the pending-hour and `.idx` fallback paths work
  as written.

Not covered by that run: the MRMS listing's yesterday fallback at 00:00 UTC, and whether the 15 s,
120 s and 300 s timeouts suffice on the deployment network.
