# radar-api

The radar-ng HTTP server (`radar api`, the tile-server image). One axum process serves the tile
pyramids, the basemap (PMTiles, styles, fonts, imagery proxy), `/api/*`, `/v1/*` and `/api/metrics`.
It replaced the Python stack's Caddy, FastAPI/uvicorn and go-pmtiles processes; routes, parameters,
JSON shapes, status codes and headers follow `docs/rust-migration/specs/spec-api.md`, so the app and
the web radar did not change.

## Public API

```rust
pub fn router(state: AppState) -> axum::Router;       // every route, edge middleware included
pub async fn serve(config: Config) -> anyhow::Result<()>; // logging, bind, graceful shutdown
AppState::from_env() -> anyhow::Result<AppState>;       // config from the environment
AppState::new(config: Config) -> anyhow::Result<AppState>;
Config::from_env() -> anyhow::Result<Config>;
Config::defaults_for(state_dir, grid_dir, tile_dir) -> Config;
```

Errors: `anyhow` at the `serve` and `AppState` boundaries. Handlers answer with the same bodies
the Python code gave, built in `respond.rs`.

## Build and test

```sh
cd rust
cargo test -p radar-api            # 78 unit tests, 26 HTTP tests (tests/http.rs)
```

The HMAC vectors in `testdata/hmac_vectors.json` were produced by the former Python
`authenticated_user`. The Rust side replays them in `workflows::auth::vector_tests`.

## Routes

Every `/api` route is a GET (`/api/livez` included). A GET-only route answers HEAD and POST with
405 `{"detail":"Method Not Allowed"}` and `Allow: GET`.

| Method | Path | Handler | Notes |
|---|---|---|---|
| GET | `/api/manifest.json` | `manifest::get_manifest` | ETag, 304, 15 s public cache, last-good fallback |
| GET | `/api/forecast/{lat}/{lon}` | `forecast::get_forecast` | Open-Meteo passthrough, 1-decimal cell cache |
| GET | `/api/inspect/{layer}/{timestamp}/{lat}/{lon}` | `grid::inspect_point` | Bilinear sample; soft `reason` on failure |
| GET | `/api/nowcast/skill` | `nowcast::get_nowcast_skill` | Skill log summary |
| GET | `/api/nowcast/{lat}/{lon}` | `nowcast::get_nowcast_point` | Point nowcast from the run files |
| GET | `/api/wind-field/{timestamp}` | `wind::get_wind_field` | `latest` accepted; u and v grids paired |
| GET | `/api/lightning` | `files::get_lightning` | Passthrough of `lightning.json` |
| GET | `/api/storms` | `files::get_storms` | Passthrough of `storms.json` |
| GET | `/api/storm-prefetch` | `prefetch::get_storm_prefetch` | Plan from `radar_storms` |
| GET | `/api/storm-prefetch/style.json` | `prefetch::get_storm_prefetch_style` | Run-scoped tile path (D1) |
| GET | `/api/tropical` | `files::get_tropical` | Passthrough of tropical data |
| GET | `/api/livez` | `health::get_livez` | Exempt from rate limit and access log |
| GET | `/api/health` | `health::get_health` | 200 ok or 503 degraded, with reasons |
| GET | `/api/basemap/style/{name}` | `basemap::get_basemap_style` | Style with origin rewritten |
| GET | `/api/metrics` | `health::get_metrics` | Prometheus text |
| GET | `/api/alerts` | `places::get_alerts` | NWS point alerts |
| GET | `/api/alerts/map` | `places::get_alerts_map` | Shaped alert features, 3-dp cache key |
| GET | `/api/geocode` | `places::get_geocode` | Geocoder, Photon fallback, settlement filter |
| GET | `/api/reverse-geocode` | `places::get_reverse_geocode` | Config check before range check |
| GET | `/api/briefing` | `briefing::get_briefing` | LLM narrative, single-flight, breaker |
| GET | `/api/alerts/explain` | `briefing::explain_alert` | LLM explainer for one NWS alert |
| POST | `/v1/push-tokens` | `workflows::post_push_token` | Bearer auth, owner check, 200 `{"registered":true}`. 503 `{"detail":"push token store is read-only"}` when `STATE_DIR` is not writable (F9) |
| DELETE | `/v1/push-tokens/{token}` | `workflows::delete_push_token` | 204 always |
| POST | `/v1/watches` | `workflows::post_watch` | Temporal start; 201, 409, 410, 503 |
| GET | `/v1/watches/{user_id}/{storm_cell_id}` | `workflows::get_watch` | Temporal query; owner check |
| DELETE | `/v1/watches/{user_id}/{storm_cell_id}` | `workflows::delete_watch` | Temporal unpin signal; 204 |

Static lane (not under `/api`):

| Path | Source | Cache-Control |
|---|---|---|
| `/tiles/*` | `ServeDir` on `TILE_DIR`, gzip precompressed | `public, max-age=86400, immutable` |
| `/basemap/styles/*` | `ServeDir` on `BASEMAP_ROOT/styles` | `public, max-age=3600` |
| `/basemap/fonts/*` | `ServeDir` on `BASEMAP_ROOT/fonts` | `public, max-age=604800` |
| `/basemap/imagery/{z}/{x}/{y}.jpg` | USGS proxy with moka cache | `public, max-age=604800` on hits and misses |
| `/basemap/tiles/{z}/{x}/{y}.mvt` | `pmtiles` crate reading `BASEMAP_PMTILES` through a positioned-read backend (F6); proxied to `BASEMAP_TILES_UPSTREAM` (default `http://basemap:8081`) when no local archive exists | `public, max-age=86400` |

ACAO `*` is set on the static lane. Errors on the static lane are overridden to
`Cache-Control: no-store` with a text body `"{code} {reason}"`. Imagery non-matches are a bare
404 with no headers.

## Caddy to axum mapping (spec 3.8)

| Caddy behavior | Port |
|---|---|
| `trusted_proxies static private_ranges` and `trusted_proxies_strict` | `client_ip::resolve`: walk `X-Forwarded-For` right to left, stop at the first untrusted address. Trusted ranges are RFC1918, loopback and `fd00::/8`, plus `TRUSTED_PROXIES`. |
| `header_up X-Client-IP {client_ip}` | Not forwarded. The client IP is computed in-process for the rate limiter (D22). |
| `file_server` on `/tiles/*` with immutable CC and ACAO | `ServeDir` nested at `/tiles`, with `SetResponseHeaderLayer::overriding` for CC and ACAO. |
| `precompressed gzip` | `ServeDir::precompressed_gzip()`. Inert in practice (D12). |
| `handle_errors`: `no-store`, `respond "{code} {reason}"` | `basemap::static_error_override` on the static lane, and `respond::caddy_error` on the top-level fallback. Responses marked `NoErrorOverride` pass through. |
| Imagery regexp and reverse proxy with z/y/x rewrite | `/basemap/imagery/{z}/{x}/{yjpg}` route, `reqwest` GET to `{USGS_IMAGERY_BASE}/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}`. Hits served from a moka cache (`IMAGERY_CACHE_MAX_BYTES`, default 128 MiB; 0 disables it). Each entry keeps the upstream `ETag` and `Last-Modified`, which are forwarded on hits and misses. |
| `handle_path /basemap/tiles/*` to the go-pmtiles sidecar | In-process `pmtiles` reader on `BASEMAP_PMTILES`. 204 for a missing tile. The file is reopened when its inode, size or mtime changes (F6). |
| `log { format json }` with probes skipped | `tracing` JSON lines from `edge::edge` with `method`, `uri`, `status`, `duration`, `user_agent`. `/api/livez` and `/api/health` are skipped. |
| Unmatched path returns 200 empty | Top-level fallback returns 404 (D13). Known templates with a trailing slash get a 307. |
| uvicorn `X-Forwarded-Proto` handling (`request.base_url` for storm-prefetch URLs) | `prefetch::request_base` reads `X-Forwarded-Proto` (default `http`) and `Host`. |

## Deviations

Items are numbered as in `docs/rust-migration/specs/spec-api.md` section 16. "Fixed" means the port does not
reproduce the Python bug. "Kept" means the port matches Python on purpose.

| ID | Decision | What the port does |
|---|---|---|
| D1 | Fixed, unverified | A closed watch can be started again. The start uses `AllowDuplicate`. Not checked against a live Temporal. |
| D2 | Fixed | "already completed" on unpin counts as success (204). |
| D3 | Fixed | A Temporal connect failure is 503 `temporal unavailable`. The client connects lazily on first use. |
| D4 | Kept | A forecast failure in the briefing still reports `llm_unavailable`. |
| D5 | Fixed | A non-JSON Open-Meteo body is 502 `upstream_unavailable` with no Cache-Control. |
| D6 | Kept | Non-finite numbers in a JSON value serialize as `null`. |
| D7 | Fixed | An inspector NaN lat or lon gives `out_of_bounds`, not 500. |
| D8 | Fixed | A Photon `osm_id` that is not an integer drops that feature. |
| D9 | Fixed | A corrupt manifest with no last-good copy no longer 500s when no storm is selected. |
| D10 | Kept | The style endpoint sends no Cache-Control and no ACAO. |
| D12 | Kept | Gzip precompression stays inert. |
| D13 | Fixed | Unmatched paths are 404. Imagery non-matches are a bare 404. |
| D14 | Fixed | The alert detail base is `NWS_ALERT_DETAIL_BASE`. Default is the NWS host. The `bad_id` prefix check still uses the NWS host, as Python does. |
| D15 | Kept | Briefing and explain responses are `Cache-Control: public`. |
| D16 | Kept | Briefing single-flight followers get no Cache-Control. |
| D17 | Fixed | A null `tracking_confidence` is coerced to 0, so the briefing does not fail. |
| D18 | Fixed | The wind-field `v` grid must match `u` in size. Short data gives `grid_read_failed`. |
| D19 | Kept | The inspector's bilinear fill quirk is preserved exactly. |
| D20 | Kept | Health still exposes `upstream_forecast`. |
| D22 | Fixed | `X-Client-IP` from the request is never trusted. The client is resolved from the peer plus `X-Forwarded-For`. |
| D23 | Kept | `Retry-After` is always `1`. |
| D25 | Kept | The style origin trusts `X-Forwarded-Host` and `X-Forwarded-Proto` without validation. |
| D26 | Kept | Forecast cache hits send the full TTL as `max-age`. |
| D28 | Changed | Place caches are shared across a 3-dp cell. Alerts come from one cached national feed, matched per 3-dp point (see below). |

Other differences:

- Disk usage for `tiles_disk` is `total - f_bavail * frsize`, as the Python code computes it.
  The spec text says `f_bfree`. This follows the code.
- The HMAC token decoder is strict. Padding and characters outside the URL-safe alphabet are
  rejected. A non-finite `exp` is rejected. Python accepted both (vector cases mark these).
- Imagery misses use `image/jpeg` as the content type.
- The imagery cache is on by default at 128 MiB (`IMAGERY_CACHE_MAX_BYTES`). Size the memory limit with that in mind, and set the variable to 0 to turn it off. Caddy did not cache imagery at all.
- **Alerts.** `/api/alerts` and `/api/alerts/map` share one national feed (`NWS_POINT_ALERTS_URL`, no query),
  fetched at most once per `ALERTS_TTL_S`. A point gets what NWS's `?point=` returns: polygon alerts by
  containment, zone alerts by the point's county, forecast zone and fire zone (`NWS_POINTS_URL`, cached a day).
  A point with no zones (`/points` 404) or a failed lookup uses the direct `?point=` query. The `title` names the
  point as NWS does. Checked against live NWS: 49 points, identical features and order.
- **Response caches** hold encoded bodies and are bounded by bytes (`*_CACHE_MAX_BYTES`). Upstream-filled caches
  (forecast, alerts) are single-flight: concurrent misses for one key make one request.
- **Compression.** `/api/*` and `/v1/*` JSON and text bodies over 1 KiB are gzip or br encoded per
  `Accept-Encoding`, with `Vary: Accept-Encoding`. Static lanes (tiles, styles, fonts, imagery, vector tiles) are
  never compressed by the server.
- **Skill snapshot.** `/api/metrics`, `/api/health` and the nowcast routes share one skill summary. It is re-read
  only when `nowcast-skill.json` changes (size, mtime, inode) or its oldest counted entry leaves the window.
- `radar-geo` is not used. The whole-grid sampler is not used. Point sampling reads four cells.
- The `/basemap/imagery` catch-all cannot be a separate route (axum rejects the overlap with the
  `{yjpg}` route). Non-matching paths fall through to the top-level fallback.

## Review fixes (docs/rust-migration/REVIEW.md, F1 to F10)

- **F1 rate limit.** The per-client token bucket applies to `/api/*` and `/v1/*` only. Static lanes (`/tiles`,
  `/basemap/styles`, `/basemap/fonts`, `/basemap/imagery`, `/basemap/tiles`) are never limited, as when Caddy
  served them. `/api/livez` stays exempt. Test: 200 tile requests all return 200, and the API lane still returns 429.
- **F5 blocking I/O.** No blocking file or lock call runs on a runtime worker. The `/api/health` status read and the
  `/api/metrics` skill-log read run on the blocking pool. Test: `/api/livez` answers while `/api/health` waits on a
  FIFO.
- **F6 basemap archive.** The archive is not memory-mapped (`MmapBackend` is gone). `FileBackend` does positioned
  reads on a blocking thread. A truncated file gives a 500 for that request and not SIGBUS. The identity (inode,
  size, mtime) is checked at most every `BASEMAP_PMTILES_RECHECK_S` seconds (default 10). A changed file is
  reopened, so a rename over the served path takes effect without a restart. A failed reopen keeps the open reader.
  Replace the archive by writing a temp file and renaming it over `BASEMAP_PMTILES`.
- **F7 metrics.** `forecast_requests_total` is counted once per `/api/forecast` request, before validation. The
  briefing path still counts through `forecast_for`. Test: four requests, including two 422s, count as four.
- **F9 push tokens.** The write is still a direct SQLite write, as in Python (`routes_workflows.py` calls `upsert`
  and does not start `RegisterPushTokenWorkflow`). If `STATE_DIR` is read-only (`access(W_OK)` is `EROFS`, `EACCES`
  or `EPERM`), the route answers 503 and logs one error line. Enabling `/v1/push-tokens` needs a writable `STATE_DIR`
  mount.
- **F10 imagery.** The cache default is 128 MiB (`IMAGERY_CACHE_MAX_BYTES`), and 0 turns it off. Each cached entry keeps
  the upstream `ETag` and `Last-Modified`, which are forwarded on hits and misses.

## Limitations and open items

- Temporal behavior is untested against a live server. Coverage is limited to the unreachable
  path (503) and the auth, validation and owner checks. D1 (`AllowDuplicate`) is unverified.
- D21 (push token writes from the API process) is decided as F9: writes stay direct and need a writable
  `STATE_DIR`. The shipped manifests mount `state` read-only, so `/v1/push-tokens` answers 503 there until that
  mount changes. The mount has not been changed.

## Configuration

Environment variables (see `src/config.rs`): `LISTEN_ADDR`, `TILE_DIR`, `GRID_DIR`, `STATE_DIR`,
`STYLE_DIR`, `BASEMAP_ROOT`, `BASEMAP_PMTILES`, `BASEMAP_PMTILES_RECHECK_S`, `BASEMAP_TILES_UPSTREAM`,
`OPEN_METEO_BASE`, `MRMS_MAX_AGE_S`,
`FORECAST_TTL_S`, `FORECAST_CACHE_MAX_BYTES` (64 MiB), `NOWCAST_POINT_CACHE_MAX_BYTES` (16 MiB),
`WIND_CACHE_MAX_BYTES` (32 MiB), `ALERTS_TTL_S`, `ALERTS_CACHE_MAX_BYTES` (16 MiB), `NWS_POINTS_URL`,
`GEOCODER_URL`, `PHOTON_URL`, `GEOCODE_TTL_S`, `GEOCODE_CACHE_MAX_BYTES` (8 MiB), `PLACES_UPSTREAM_TIMEOUT_S`,
`LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL`, `LLM_TIMEOUT_S`, `LLM_MAX_CONCURRENCY`,
`LLM_BREAKER_FAILURES`, `LLM_BREAKER_COOLDOWN_S`, `BRIEFING_TTL_S`, `NWS_POINT_ALERTS_URL`,
`NWS_ALERT_DETAIL_BASE`, `NWS_USER_AGENT`, `USGS_IMAGERY_BASE`, `IMAGERY_CACHE_MAX_BYTES`,
`API_RATE_LIMIT_RPS`, `API_RATE_LIMIT_BURST`, `TRUSTED_PROXIES`, `DISABLE_WORKFLOW_ROUTES`,
`WORKFLOW_API_SIGNING_KEY` (or `WORKFLOW_AUTH_SIGNING_KEY`), `TEMPORAL_ADDRESS`,
`TEMPORAL_NAMESPACE`, `TEMPORAL_ALERTS_TASK_QUEUE`. The old `*_CACHE_MAX_ENTRIES` variables are ignored.
