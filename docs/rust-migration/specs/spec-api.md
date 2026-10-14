# radar-ng API: behavioral porting spec (backend/api to Rust)

Source snapshot: `/mnt/user-data/uploads/radar-ng` (read-only). Scope: every route a client can reach through the public tile-server port (Caddy, `:8080`), plus the uvicorn/FastAPI app behind it. Target: a Rust service that a client cannot distinguish from the current stack for these paths. The React Native app (`frontend/`) and the web app (`web/`) must not change.

Verification tags used throughout:
- **[V]** verified by running the test suite or a probe against the Python code in this session.
- **[R]** read from source; not executed.
- **[I]** inferred from code plus library semantics; confirm against a live system before relying on it.

Verification basis: all 68 tests in `backend/api/api/test_*.py`, `backend/shared/test_nowcast_skill.py` and `backend/shared/test_manifest.py` pass (68 passed) in a scratch copy, using fastapi 0.143.0, starlette 1.8.0, httpx 0.28.1, temporalio 1.30.0, numpy, pytest. `backend/api/api/requirements.txt` pins `fastapi>=0.115` with no upper bound, so production resolves whatever pip picks at image build time. Everything below is for the tested versions. Probe scripts used for [V] claims are in the scratch directory `probe/probe1.py` through `probe6.py` (ephemeral).

---

## 0. Read this first

1. **Topology.** Clients speak only to Caddy on `:8080`. Caddy reverse-proxies `/api/*` and `/v1/*` to uvicorn on `127.0.0.1:8000` (one process, one worker, `--no-access-log`). Tiles, basemap styles, fonts, USGS imagery and PMTiles never reach Python except the PMTiles and imagery proxies, which Caddy forwards itself. [R]
2. **JSON encoding.** Every JSON body is produced by Starlette `JSONResponse`: compact separators `,` and `:`, UTF-8 (not ASCII-escaped), `allow_nan=False`. Any NaN or Infinity in a body raises and becomes a 500 `Internal Server Error` (text/plain). [V: `ValueError: Out of range float values are not JSON compliant`]
3. **No CORS and no compression at origin on `/api` or `/v1`.** The Caddyfile has no `encode` directive and the app has no CORS or GZip middleware. The `precompressed gzip` directive on `/tiles/*` is inert: nothing in the ingest or tiler writes `.gz` siblings. [R]
4. **Error envelopes.** `HTTPException` gives `{"detail": "<msg>"}`. Request validation gives 422 `{"detail": [ {type, loc, msg, input[, ctx]} ]}`. Custom failures use `{"error": "<code>"}` (forecast, alerts, geocode). The LLM endpoints always answer 200 with `{"available": false, "reason": "<code>"}`. [V]
5. **Two failure philosophies.** Data endpoints fail soft with 200 and an `ok:false` or empty body (inspect, wind-field, lightning, storms, tropical, storm-prefetch, nowcast). Upstream proxies fail with 502 (`{"error":"upstream_unavailable"}`), and the cache rule is per endpoint (alerts and geocode are `no-store`; forecast has no `Cache-Control` on 502). [V]
6. **Caches are in-process and per replica.** Nothing is shared between replicas. The state directory (`STATE_DIR`) is the only cross-process channel; the worker writes it and the API reads it.
7. **Blocking rule.** Every handler that touches the filesystem is a sync `def` (threadpool). Only `/api/forecast`, `/api/livez`, `/api/alerts*`, `/api/geocode`, `/api/reverse-geocode`, `/api/briefing`, `/api/alerts/explain`, and the `/v1/*` handlers are `async`. `/api/livez` must never touch the filesystem. These rules are enforced by `test_server_concurrency.py`. [R][V]

---

## 1. Global conventions

### 1.1 Numeric rules (must match exactly)
- **Python `round(x, n)`** returns the double nearest the decimal result of correctly rounded `x`, ties to even on exact binary ties. Rust equivalent for coordinate and ratio rounding: `format!("{:.*}", n, x).parse::<f64>()`. [V: `round(42.0,3)` repr `42.0`]
- **Python `round(x)` (int)** is round-half-to-even: `round(2.5) == 2`, `round(0.5) == 0`. Rust: use `f64::round_ties_even`, never `f64::round`. [V]
- **Float repr in strings** (used only where a string is built): Python `repr(42.0)` is `42.0`; `repr(0.00001)` is `1e-05`. The NWS `point=` value is built from these, so a Rust port should format with `{:?}` (gives `42.0`) for values in normal ranges. [V]
- **Path and query floats** are parsed by FastAPI with Python `float()`. That accepts `nan`, `inf`, `infinity`, leading `+`, and underscores (`float("1_0") == 10.0`). NaN and Inf then fail range checks and return 422. A Rust parser should reject underscores and whitespace; no client sends them. [V]
- **Integer query params** (`zoom`, `limit`) use Pydantic int parsing: `"6"` ok, `"6.5"` is a 422. [V for 6.5 class via FastAPI rules; R for detail]

### 1.2 Validation order inside a handler
FastAPI resolves parameters first (422 with array body), then the handler body runs. Handlers with a configuration check (places, briefing, explain) answer their 503/not_configured before their own range checks. Example: `/api/reverse-geocode?lat=abc` gives 422 array; `/api/reverse-geocode?lat=91&lon=0` with no geocoder configured gives 503 `geocoding_not_configured`, not 422. [R]

### 1.3 Time formats
- Manifest, frame, and alert timestamps are ISO-8601 with an explicit offset, normally `+00:00`, e.g. `2026-10-13T12:00:00+00:00`. The frontend validator requires an offset and sorted, unique timestamps (see section 13.1). [R]
- `checked_at`, `generated_at`, `updated_at` in API bodies come from `datetime.now(timezone.utc).isoformat()`. Microseconds are emitted except when zero. [R]
- Wall-clock `time.time()` is used for TTL checks on caches; the breaker uses `time.monotonic()`. [R]

### 1.4 Headers common to all responses
- Starlette adds `content-length`. Unknown/unmatched paths return Starlette's 404 `{"detail":"Not Found"}`. GET-only routes answer HEAD and OPTIONS with 405 `{"detail":"Method Not Allowed"}` and `Allow: GET` [V].
- Trailing-slash variants of an existing route redirect (Starlette `redirect_slashes`, 307) [R].
- No `Server`, `Vary`, `ETag` (except manifest), `Last-Modified`, `Content-Language`. No `X-Request-Id`.
- Starlette JSON content type is `application/json` with no charset. PlainText (metrics) is `text/plain; charset=utf-8`. [R]

---

## 2. Route inventory

Legend: sync = `def` (threadpool); async = `async def`. "CC" = Cache-Control on success. All paths are GET unless noted. The rate limiter (section 4) applies to every route below except `/api/livez`.

| # | Method | Path | Handler | Kind | Success | Common error codes | CC on success |
|---|---|---|---|---|---|---|---|
| 1 | GET | `/api/manifest.json` | `get_manifest` | sync | 200 JSON, or 304 | 503 `manifest unavailable` | `public, max-age=15` + ETag |
| 2 | GET | `/api/forecast/{lat}/{lon}` | `get_forecast` | async | 200 Open-Meteo passthrough | 422 range, 422 parse, 502 `upstream_unavailable` | `public, max-age=<FORECAST_TTL_S>` |
| 3 | GET | `/api/inspect/{layer}/{timestamp}/{lat}/{lon}` | `inspect_point` | sync | 200 always | none (soft fail) | none |
| 4 | GET | `/api/nowcast/{lat}/{lon}` | `nowcast_point` | sync | 200 (`ok`/`degraded`/`unavailable`) | 422 range | 30 (ok), 15 (unavailable) |
| 5 | GET | `/api/nowcast/skill` | `nowcast_skill_summary` | sync | 200 | none | `public, max-age=30` |
| 6 | GET | `/api/wind-field/{timestamp}` | `wind_field` | sync | 200 (`ok` true or false) | none (soft fail) | `public, max-age=300` on ok only |
| 7 | GET | `/api/lightning` | `lightning` | sync | 200 | none | `public, max-age=10` |
| 8 | GET | `/api/storms` | `storms` | sync | 200 | none | `public, max-age=30` |
| 9 | GET | `/api/storm-prefetch` | `storm_prefetch` | sync | 200 plan | 422 lat/lon/zoom/palette | `public, max-age=30` |
| 10 | GET | `/api/storm-prefetch/style.json` | `storm_prefetch_style` | sync | 200 style | 422 layer/palette/zoom/timestamp | `public, max-age=30` |
| 11 | GET | `/api/tropical` | `tropical` | sync | 200 | none | `public, max-age=180` |
| 12 | GET | `/api/livez` | `livez` | async | 200 `{"status":"ok"}` | none | none |
| 13 | GET | `/api/health` | `health` | sync | 200 ok / 503 degraded | 503 `manifest unavailable` (HTTPException body) | none |
| 14 | GET | `/api/basemap/style/{name}` | `get_basemap_style` | sync | 200 style JSON | 400 `invalid style name`, 404 `style not found` | none |
| 15 | GET | `/api/metrics` | `metrics` | sync | 200 text | 503 `manifest unavailable` | none |
| 16 | GET | `/api/alerts` | `get_alerts` | async | 200 NWS body | 422, 502 (no-store) | `public, max-age=60` |
| 17 | GET | `/api/alerts/map` | `get_map_alerts` | async | 200 FeatureCollection | 502 (no-store) | `public, max-age=60` |
| 18 | GET | `/api/geocode` | `geocode` | async | 200 `{"results":[...]}` | 422, 502 (no-store), 503 (no-store) | `public, max-age=3600` |
| 19 | GET | `/api/reverse-geocode` | `reverse_geocode` | async | 200 `{"place":...}` | 422, 502 (no-store), 503 (no-store) | `public, max-age=3600` |
| 20 | GET | `/api/briefing` | `get_briefing` | async | 200 `available:true` | 422 (params) | `public, max-age=120`; no-store when unavailable |
| 21 | GET | `/api/alerts/explain` | `explain_alert` | async | 200 `available:true` | 422 (id length) | `public, max-age=600`; no-store when unavailable |
| 22 | POST | `/v1/push-tokens` | `register_push_token` | async | 200 `{"registered":true}` | 401, 404, 422, 500 (sqlite) | none |
| 23 | DELETE | `/v1/push-tokens/{token}` | `delete_push_token` | async | 204 | 401, 500 (sqlite) | none |
| 24 | POST | `/v1/watches` | `start_watch` | async | 201 `{workflow_id, run_id}` | 401, 404, 409, 422, 503, 500 (connect) | none |
| 25 | DELETE | `/v1/watches/{user_id}/{storm_cell_id}` | `stop_watch` | async | 204 | 401, 404, 503 | none |
| 26 | GET | `/v1/watches/{user_id}/{storm_cell_id}` | `get_watch` | async | 200 WatchState | 401, 404, 410, 503 | none |

Workflow routes exist only when `DISABLE_WORKFLOW_ROUTES != "1"` (section 6). When disabled, `/v1/*` gets FastAPI 404 `{"detail":"Not Found"}` through Caddy. [R]

### 2.1 Caddy-only routes (never reach Python unless noted)

| # | Matcher | Behavior | CC | ACAO |
|---|---|---|---|---|
| C1 | `/tiles/*` | static file from `/data/tiles/...` (root `/data`, path kept) | `public, max-age=86400, immutable` | `*` |
| C2 | `/basemap/styles/*` | static from `/srv/basemap/styles/...` (prefix `/basemap` stripped) | `public, max-age=3600` | `*` |
| C3 | `/basemap/fonts/*` | static from `/srv/basemap/fonts/...` | `public, max-age=604800` | `*` |
| C4 | regexp `^/basemap/imagery/([0-9]|1[0-6])/([0-9]+)/([0-9]+)\.jpg$` | reverse proxy to `https://basemap.nationalmap.gov` (USGS, see 3.4) | `public, max-age=604800` | `*` |
| C5 | `/basemap/imagery/*` (other) | `respond 404`, empty body, no headers | none | none |
| C6 | `/basemap/tiles/*` (handle_path) | reverse proxy to `basemap:8081` as `/basemap/{z}/{x}/{y}.mvt` | `public, max-age=86400` | `*` |
| C7 | `/api/*` | reverse proxy to `127.0.0.1:8000`, sets `X-Client-IP` | from app | none |
| C8 | `/v1/*` | reverse proxy to `127.0.0.1:8000`, sets `X-Client-IP` | from app | none |
| C9 | unmatched | no handler; answers 200 with empty body (per Caddyfile comment on `/v1`) | none | none |

---

## 3. Caddy (front proxy) exact behavior

Source: `backend/api/Caddyfile`, `backend/api/start.sh`, `backend/api/Dockerfile`. Caddy 2.11.4 (pinned image tag). [R]

### 3.1 Global options
```
servers {
    trusted_proxies static private_ranges
    trusted_proxies_strict
}
```
- `private_ranges` is Caddy's shorthand for RFC1918 IPv4 (10/8, 172.16/12, 192.168/16), loopback (127/8, ::1) and fd00::/8. [R]
- `trusted_proxies_strict` means `client_ip` is resolved by walking `X-Forwarded-For` right to left and stopping at the first address that is not trusted. If the direct peer is untrusted, `client_ip` is the peer. [R]

### 3.2 Site and listener
- Single site `:8080`, plain HTTP. TLS is terminated upstream (Cloudflare, then cloudflared). Caddy's own TLS is not used. [R]
- `start.sh` launches uvicorn in the background, waits up to 30 s for it to accept TCP on `127.0.0.1:8000`, then runs `caddy run` in the foreground. If uvicorn dies, Caddy keeps running and `/api/*` answers 502. That is why the liveness probe goes through Caddy. [R]

### 3.3 Headers and reverse proxy details
- `reverse_proxy 127.0.0.1:8000 { header_up X-Client-IP {client_ip} }` on `/api/*` and `/v1/*`. `header_up` replaces any incoming `X-Client-IP`, so clients cannot forge it on these paths. [R]
- Caddy's default `reverse_proxy` behavior also applies: original `Host` is preserved; `X-Forwarded-For`, `X-Forwarded-Proto` and (I believe) `X-Forwarded-Host` are set. Exact header set to verify on 2.11.4. [I]
- No timeouts are configured on `reverse_proxy`; Caddy defaults apply. [R]
- uvicorn runs with its default proxy-header handling (`proxy_headers=True`, trusted peers `FORWARDED_ALLOW_IPS` default `127.0.0.1,::1`). So when Caddy sends `X-Forwarded-Proto: https`, uvicorn sets the request scheme to `https`, and `request.base_url` (used by storm-prefetch URLs) becomes `https://<Host>`. [V for uvicorn default source; R for the Caddy header behavior]

### 3.4 The imagery proxy (C4)
- Regexp groups: `{re.tile.1}` = z, `{re.tile.2}` = x, `{re.tile.3}` = y.
- Rewrite to `/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}` (ArcGIS order z/y/x).
- `header_up Host {upstream_hostport}`; `header_down -Cache-Control`, `-Set-Cookie`, `-X-Cache`, `-Via`; then Caddy adds `Cache-Control: public, max-age=604800` and ACAO `*`.
- Caddy does not cache imagery. Every client request goes to USGS. A Rust port may add a cache (recommended, section 16).

### 3.5 Error handling (`handle_errors`)
- Applies to any error from any handle (including file_server 404 and reverse-proxy 502/503/504 when uvicorn or the basemap sidecar is down).
- Sets `Cache-Control: no-store` and responds `respond "{err.status_code} {err.status_text}"` with the same status. Body is text/plain, e.g. `404 Not Found`, `502 Bad Gateway`.
- This overrides the per-path `Cache-Control` set earlier, which is the reason tile misses are not cached. [R]
- Note: `respond 404` in C5 is not an error and does not go through `handle_errors`; it has no Cache-Control at all. [R]

### 3.6 Logging
- `log { output stdout; format json }`. `@probes path /api/livez /api/health` is excluded via `log_skip`. Loki parses `status`, URI, duration and `User-Agent`, which the Grafana dashboard uses to tell the app, widget, Watch and CarPlay apart. [R]
- `docs/privacy.md` describes a console-format Caddy log; the Caddyfile says JSON. The Caddyfile is authoritative. [R]

### 3.7 Things Caddy does NOT do (do not assume them in Rust)
- No gzip/zstd/br encoding at origin (see section 0.3).
- No CORS headers on `/api/*` or `/v1/*`.
- No caching of any proxied response.
- No request body size limit, no header size limit beyond Go defaults.
- No rate limiting. The only limiter is in Python (section 4).

### 3.8 Rust replacement mapping
| Caddy behavior | Axum / tower-http equivalent |
|---|---|
| `file_server` for `/tiles/*` with `immutable` CC and ACAO `*` | `ServeDir::new(TILE_DIR)` nested at `/tiles`; add headers on 200/206/304; ACAO `*` |
| `precompressed gzip` | `ServeDir::precompressed_gzip()` (harmless, matches current behavior) |
| error override to `no-store` plain text | middleware that rewrites status>=400 to `no-store`, text/plain `"{code} {reason}"` |
| `/basemap/styles`, `/basemap/fonts` static | `ServeDir` on `/srv/basemap/styles` and `/srv/basemap/fonts` with the two CC values |
| imagery regexp and reverse proxy | axum route with two `u8/u64` path parts, reqwest GET to USGS with the z/y/x rewrite, header filtering |
| `handle_path /basemap/tiles/*` reverse proxy to `basemap:8081` | reqwest proxy, or keep the go-pmtiles sidecar and proxy to it (recommended) |
| `header_up X-Client-IP {client_ip}` | compute `client_ip` from socket peer plus XFF using the strict right-to-left rule, then forward header internally (or use in-process) |
| JSON access log, probe skip | `tower-http` `TraceLayer` with JSON `tracing-subscriber`; skip `/api/livez` and `/api/health`; keep field names `status`, `uri`, `duration`, `user_agent` for Loki |
| unmatched path returns 200 empty | recommend 404 (divergence, section 16) |

### 3.9 Recommended cutover
Phase 1: keep Caddy as-is and point its `/api/*` and `/v1/*` upstream at the Rust binary on `127.0.0.1:8000`. This keeps every header and the error override byte-for-byte and needs no client change. Phase 2 (optional): fold Caddy into the Rust binary once header parity tests pass (section 18).

---

## 4. Global middleware: rate limit, client key, metrics

Registered with `@app.middleware("http")`, so it wraps every request, including 404s and 405s. Single-process state; no locks across processes.

### 4.1 Client key
```
client_key(req):
  v = req.header("x-client-ip").trim()
  if v != "": return v
  f = req.header("x-forwarded-for")
  if f != "": return (f.rsplit(",", 1)[-1].trim()) or "unknown"
  return req.client.host or "unknown"
```
- Trust model: `X-Client-IP` is set by Caddy on `/api/*` and `/v1/*` only. The app binds `127.0.0.1` only (start.sh), so the header cannot be forged from outside the pod. If the app were exposed directly, the header would be forgeable. [R]
- The XFF fallback takes the rightmost hop (`rsplit` once). The leftmost is client-controlled. [V: tests `test_client_key_*`]

### 4.2 Token bucket (exact algorithm)
Constants: `RPS = API_RATE_LIMIT_RPS` (default 20.0), `BURST = API_RATE_LIMIT_BURST` (default 60.0). Store: LRU map `client -> (tokens: f64, updated: monotonic seconds)`, capacity 4096.
```
now = monotonic()
(tokens, updated) = store.pop(client) or (BURST, now)      # pop, then re-insert = LRU touch
tokens = min(BURST, tokens + max(0, now - updated) * RPS)
limited = path != "/api/livez" and tokens < 1.0
if path != "/api/livez" and not limited: tokens -= 1.0      # 429s do not consume
store[client] = (tokens, now)                               # stored BEFORE the handler runs
while len(store) > 4096: evict least-recently touched
if limited:
    metrics.rate_limited_requests_total += 1
    respond 429 JSON {"error":"rate_limited"}, headers Retry-After: 1
else:
    response = await next()
record(method, route_template_or_"unmatched", status) += 1
record_duration(method, route_template_or_"unmatched") += elapsed_seconds
```
- Verified: with BURST=3 and RPS=0, five `/api/metrics` calls gave 200,200,200,429,429; the 429 body was `{"error":"rate_limited"}` with `Retry-After: 1` and `application/json` [V]. With the same settings, five `/api/livez` calls all returned 200 [V].
- `Retry-After` is the constant `1` regardless of actual refill time. [R]
- The duration is measured around the whole middleware, including the 429 path. [R]
- Route label is the matched route template (e.g. `/api/forecast/{lat}/{lon}`), `unmatched` otherwise. Raw URLs are never labels. [V: `test_route_label_*`]

### 4.3 What is not rate limited
- Everything Caddy serves directly (tiles, styles, fonts, imagery, PMTiles). [R]
- Nothing is limited per user id, per route, or globally.

---

## 5. Endpoint specifications

Each subsection lists: inputs and validation, success body (every key with type and example), status and error bodies, headers, caches, upstream, and algorithm notes that a port must reproduce.

### 5.1 `GET /api/manifest.json` (sync)
**Input:** optional header `If-None-Match`. No query params.

**Behavior:** returns `read_manifest_file(STATE_DIR)` as JSON. The file is written by ingest/cleanup via `update_manifest_file` / `replace_layer_manifest` (atomic temp+rename under `flock` on `manifest.lock`).

**Normalization applied on read** [R]:
- Missing file: returns an empty manifest `{"schema_version":2,"layers":{},"tile_url_template":"/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png","tile_url_template_legacy":"/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png","updated_at":<now>}`.
- Unreadable or corrupt JSON: raises (`OSError` or `ValueError`). The API then serves the last good copy or 503 (see below).
- Non-dict document, or `layers` not a dict: treated as empty manifest.
- Missing `tile_url_template`, `tile_url_template_legacy`, `updated_at`, `schema_version`: defaults added (`schema_version` default 1).
- A layer without a `frames` list gets `frames` synthesized from its `timestamps` (`{timestamp, path: timestamp, palettes: layer.palettes or ["classic"]}`).

**Success body (example, abbreviated):**
```json
{"schema_version":2,
 "layers":{
   "radar":{"timestamps":["2026-10-13T11:58:00+00:00","2026-10-13T12:00:00+00:00"],
            "frames":[{"timestamp":"2026-10-13T11:58:00+00:00","path":"2026-10-13T11:58:00+00:00","palettes":["classic","muted","vivid"]}],
            "latest":"2026-10-13T12:00:00+00:00","palettes":["classic","muted","vivid"]},
   "nowcast":{"timestamps":[...],"frames":[{"timestamp":"...","path":"...","palettes":[...],"lead_minutes":5,"spatial_resolution_km":2.22,"grid_key":"runs/<run>/<ts>"}],
              "run_id":"2026-10-13T12:00:00+00:00","method":"pysteps-sprog","horizon_minutes":60,"step_minutes":5,"latest":"..."}},
 "tile_url_template":"/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
 "tile_url_template_legacy":"/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
 "updated_at":"2026-10-13T12:00:05.123456+00:00"}
```
- Layer-level keys that are not written by the schema code (`title`, `kind`, `complete`, `method`, `run_id`, `horizon_minutes`, `step_minutes`, layer-level `grid_key` if any) pass through unchanged because `layer_metadata` is merged in. [R]
- Frame keys pass through as stored (`source`, `kind`, `issued_at`, `lead_minutes`, `spatial_resolution_km`, `max_zoom`, `grid_key`, `palettes`).
- `frames` is sorted by timestamp string; `timestamps` mirrors it; `latest` is the last timestamp; layer `palettes` is the sorted intersection of frame palettes.

**Serialization:** `JSONResponse(body).body` (compact). This encoded byte string is what the ETag is computed over.

**Headers on 200:** `Cache-Control: public, max-age=15`, `ETag: "<hex>"`, `Content-Type: application/json`.
**ETag:** `'"' + blake2b(encoded, digest_size=12).hexdigest() + '"'`. 24 hex chars. [V: observed `"a88eae051e663ec24d4f0827"`]

**304 rule:** if `If-None-Match` is present and any comma-separated candidate (after trimming, with a leading `W/` removed) equals the ETag, or equals `*`, respond 304 with empty body and headers `ETag` and `Cache-Control` (no Content-Type body). [V: `"stale", <etag>` matches; `W/<etag>` matches; `"stale"` alone does not]

**Errors:**
- 503 `{"detail":"manifest unavailable"}` when the read fails and no last-good copy exists (in-process last-good body is kept from the last successful read, with no TTL).
- Corrupt file with a last-good copy: 200 with the last-good body (logged as warning). [V: `test_build_manifest_serves_last_good_copy_then_503`]

**Cache:** `_manifest_cache` single entry, TTL 15 s (wall clock). Holds parsed dict, encoded bytes and ETag. Re-read at most once per TTL. [V: one read across two GETs and one nowcast call]

### 5.2 `GET /api/forecast/{lat}/{lon}` (async)
**Path:** `lat`, `lon` floats. Range: `-90 <= lat <= 90`, `-180 <= lon <= 180`, else 422 `{"detail":"lat/lon out of range"}`. Non-numeric: FastAPI 422 array (`float_parsing`). [V]

**Rounding:** `grid_lat = round(lat, 1)`, `grid_lon = round(lon, 1)` (about 11 km cells). The upstream request and the cache key both use the rounded values. [V: `42.96 -> 43.0`, `-85.67 -> -85.7`]

**Cache:** key `f"{grid_lat},{grid_lon}"` (Python repr), value `(insert_time, body)`. TTL `FORECAST_TTL_S` (default 900). Max entries `FORECAST_CACHE_MAX_ENTRIES` (default 512), LRU by touch on hit, evict oldest on insert. Expired entry is popped on access. [R][V: second request in same cell served from cache, one upstream call]

**Upstream:** `GET OPEN_METEO_BASE` with query params (order as listed, verified):
```
latitude=<grid_lat>&longitude=<grid_lon>
current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m,dew_point_2m,surface_pressure,precipitation
hourly=temperature_2m,precipitation,precipitation_probability,weather_code,wind_speed_10m,relative_humidity_2m,apparent_temperature
minutely_15=temperature_2m,relative_humidity_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m
daily=temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,precipitation_probability_max,uv_index_max,sunrise,sunset
temperature_unit=fahrenheit&wind_speed_unit=mph&precipitation_unit=inch&timezone=auto&forecast_days=7
```
Commas are percent-encoded (`%2C`) by httpx. No custom `User-Agent` (httpx default). Client timeout 15 s total; pool `max_connections=100`, `max_keepalive_connections=20`. No retries. [V for URL]

**Success:** the Open-Meteo JSON is decoded and re-encoded unchanged (passthrough). Keys the app reads: `latitude`, `longitude`, `utc_offset_seconds`, `current{time, temperature_2m, relative_humidity_2m, apparent_temperature, weather_code, wind_speed_10m, wind_direction_10m, wind_gusts_10m, dew_point_2m, surface_pressure}`, `hourly{time, temperature_2m, precipitation_probability, precipitation, weather_code, wind_speed_10m, visibility?}`, `daily{time, temperature_2m_max, temperature_2m_min, weather_code, precipitation_sum, precipitation_probability_max, uv_index_max, sunrise, sunset}`, `minutely_15{time, precipitation, precipitation_probability?}` (see `frontend/src/types/weather.ts`). Null values are normal (model horizon).

**Errors:**
- Upstream non-2xx, connect error, timeout, or body that is not HTTP-level failure but `httpx.HTTPError`: 502 `{"error":"upstream_unavailable"}`, **no Cache-Control**, `forecast_upstream_errors_total += 1`. [V: closed port gave 502 with no header]
- Upstream 200 with non-JSON body: `resp.json()` raises `JSONDecodeError`, which is not caught, so the response is **500**. [V: raised JSONDecodeError under TestClient] Recommended port behavior: 502 (divergence, section 16).

**Headers on 200:** `Cache-Control: public, max-age=<FORECAST_TTL_S>` (not age-adjusted on cache hits). [V]

**Metrics:** `forecast_requests_total` incremented before validation (so 422s count). `forecast_cache_hits_total` on hit.

### 5.3 `GET /api/inspect/{layer}/{timestamp}/{lat}/{lon}` (sync)
**Behavior:** point sample of a legacy (timestamp-keyed) grid; see section 9.1 for the exact read algorithm.

**Path:** `layer`, `timestamp` (URL-decoded by the router; `+` must arrive as `%2B`), `lat`, `lon` floats. No range validation in the handler. Out-of-range coordinates return `out_of_bounds`, not 422. NaN lat or lon crashes (section 16). [V: lat 44.0 gave `out_of_bounds`]

**Success (always 200):**
```json
{"ok": true, "value": 25.0, "unit": "dBZ", "layer": "radar", "timestamp": "2026-07-15T16:05:00+00:00", "lat": 42.5, "lon": -85.5}
```
- `value` may be `null` with `ok: true` and `reason: "no_data"`: `{"ok":true,"value":null,"unit":"dBZ","reason":"no_data"}`. (no layer/timestamp keys on this branch) [R]
- `timestamp` echoes the decoded path string exactly. `lat` and `lon` echo the request values (not rounded). [V]

**Failure shapes (200, no Cache-Control):**
- `{"ok": false, "reason": "grid_missing", "layer": "<layer>", "timestamp": "<ts>"}` when `meta.json` is absent or the name sanitizes to an invalid path. [V]
- `{"ok": false, "reason": "out_of_bounds", "lat": <lat>, "lon": <lon>}`. [V]
- `{"ok": false, "reason": "meta_invalid"}` when metadata JSON is unreadable, lacks a required key, or the data file is missing.
- `{"ok": false, "reason": "read_error"}` when the data file read raises `OSError`.

**Headers:** no Cache-Control on any branch. [V]

### 5.4 `GET /api/nowcast/{lat}/{lon}` (sync)
**Path:** `lat`, `lon` floats, range-checked to 422 `{"detail":"lat/lon out of range"}`. Frontend rounds to 2 decimals; the server rounds to 3.

**Flow** [R][V where noted]:
1. Increment `nowcast_point_requests_total`.
2. `manifest = _cached_manifest()`; `layer = manifest.layers.nowcast`.
3. If `layer` is not a dict or has no `frames` (non-empty): respond `unavailable` branch A (below). Status file read: `STATE_DIR/nowcast-status.json`; `reason = status.reason` if the key exists else `"no_frames"`; `detail = status.detail` (or null).
4. `run_id = str(layer.run_id or layer.latest or "unknown")`; `grid_lat = round(lat,3)`; `grid_lon = round(lon,3)`; cache key `(run_id, grid_lat, grid_lon)`.
5. Cache hit: respond with cached body plus fresh `skill`, CC 30. Counts `nowcast_point_cache_hits_total`.
6. Otherwise for each frame (skip non-dict or no `timestamp`): `grid_key = frame.grid_key` if it is a string, else None. Sample via section 9.2. `ok:false` counts `missing += 1` (and `out_of_bounds` if that reason). `ok:true` with value None gives `dbz: null` and `precipitation_mm_h: 0.0` (not counted as missing). Otherwise append point `{timestamp, lead_minutes: frame.lead_minutes (raw, may be null), dbz, precipitation_mm_h}`.
7. No points: `unavailable` branch B.
8. Otherwise build `ok`/`degraded` body (below), store in cache (no TTL, LRU by `NOWCAST_POINT_CACHE_MAX_ENTRIES`, default 1024), and respond with `skill`.

**Rain rate** (Marshall-Palmer): `dbz < 5 -> 0.0`; else `Z = 10^(dbz/10)`, `R = (Z/200)^(1/1.6)`, clamped to `[0, 300]`. Only applied when `dbz` is not null. [R]

**Success body (`ok` or `degraded`):**
```json
{"status":"ok","source":"mrms-nowcast","method":"pysteps-sprog","issued_at":"2026-07-15T16:00:00+00:00",
 "horizon_minutes":60,"step_minutes":5,"spatial_resolution_km":2.22,
 "latitude":42.5,"longitude":-85.5,
 "points":[{"timestamp":"2026-07-15T16:05:00+00:00","lead_minutes":5,"dbz":30.0,"precipitation_mm_h":1.23}],
 "skill":{"lead_minutes":30,"window_hours":24,"runs":8,"pod":0.8,"far":0.2,"csi":0.6,"persistence_csi":0.4} }
```
- `status` is `ok` when zero frames were missing, `degraded` when any were missing.
- `method`, `issued_at` come from the manifest and may be null. `horizon_minutes` default 60, `step_minutes` default 5. `spatial_resolution_km` is the first non-null `spatial_resolution_km` across frames, else null.
- `skill` is the headline object or null (section 5.5).
- Points are in manifest frame order (already sorted by timestamp in the manifest).

**Unavailable branch A** (no nowcast frames) `200`, CC 15, **no `skill` key**:
```json
{"status":"unavailable","reason":"no_frames","detail":null,"points":[]}
```
Reason and detail come from `nowcast-status.json` when present.

**Unavailable branch B** (frames exist but no sample succeeded) `200`, CC 15, **no `skill` key**:
```json
{"status":"unavailable","reason":"grids_warming_up","issued_at":"2026-07-15T16:00:00+00:00","points":[]}
```
`reason` is `out_of_bounds` only when every missing frame was out of bounds; otherwise `grids_warming_up`. `issued_at` is the raw manifest `run_id` (may be null). [V: exact body `test_nowcast_point_reports_warming_grids`]

**Cache-hit path:** body is cached without `skill` and re-merged with the current headline on every response; CC 30. [V: `test_nowcast_point_carries_the_skill_headline`]

### 5.5 `GET /api/nowcast/skill` (sync)
**Behavior:** rolling verification summary. Computed from `STATE_DIR/nowcast-skill.json` (section 13.6) via `backend/shared/nowcast_skill.py`. Cached 15 s (`_skill_cache`, TTL reuses `_MANIFEST_TTL_S`).

**Body:**
```json
{"available": true, "window_hours": 24, "thresholds_dbz": {"rain": 20.0, "heavy": 35.0},
 "runs_scored": 8, "updated_at": "2026-10-13T12:00:00+00:00",
 "by_lead": [
   {"lead_minutes": 30, "runs": 8,
    "rain":  {"hits":8,"misses":2,"false_alarms":2,"pod":0.8,"far":0.2,"csi":0.667,"bias":1.0,"persistence_csi":0.25},
    "heavy": {"hits":0,"misses":0,"false_alarms":0,"pod":null,"far":null,"csi":null,"bias":null,"persistence_csi":null}}],
 "headline": {"lead_minutes":30,"window_hours":24,"runs":8,"pod":0.8,"far":0.2,"csi":0.667,"persistence_csi":0.25}}
```
- Empty log: `{"available": false, ..., "by_lead": [], "headline": null, "reason": "no_scored_runs", "updated_at": null, "runs_scored": 0}`.
- Reason values: `no_scored_runs`, `warming_up` (headline lead has fewer than `NOWCAST_SKILL_MIN_RUNS` runs), `no_rain_observed` (rain POD undefined). `reason` key is absent when available. [R]
- Ratio rounding: `round(num/den, 3)` (Python), `null` when denominator is 0. `bias = (hits+false_alarms)/(hits+misses)`. `persistence_csi` is null when persistence hits+misses is 0.
- Headline lead: the bucket whose `lead_minutes` is closest to `NOWCAST_SKILL_HEADLINE_LEAD_MIN` (ties to the lower lead).
- `runs` = count of distinct anchors per lead bucket.
- Entries older than the window (`NOWCAST_SKILL_WINDOW_HOURS`) are ignored.

**Headers:** `Cache-Control: public, max-age=30`. [V]

### 5.6 `GET /api/wind-field/{timestamp}` (sync)
**Path:** `timestamp` is sanitized by keeping only alnum and `:-_+.T` characters. The literal `latest` (case-insensitive) selects the newest common timestamp.

**Resolution** [R][V]:
1. `u_meta = GRID_DIR/wind_u/<ts>.meta.json`, `v_meta` likewise.
2. If `ts == "latest"` or either meta is missing: take the intersection of stems of `wind_u/*.meta.json` and `wind_v/*.meta.json`, sorted lexicographically; take the last. If empty: respond `200 {"ok": false, "reason": "grid_missing", "timestamp": "<raw path value>"}` (no Cache-Control). [V: `{"ok":false,"reason":"grid_missing","timestamp":"latest"}`]
3. Cache lookup by resolved `ts` (TTL 300 s hard-coded, max `WIND_CACHE_MAX_ENTRIES` default 48, LRU). Hit: CC 300.
4. Read both metas and both `.bin` files (whole file). Any `OSError`, missing key, or `ValueError` gives `200 {"ok": false, "reason": "grid_read_failed"}` (no CC).

**Decimation** [R]: target 240 by 120. `sx = max(1, W // 240)`, `sy = max(1, H // 120)`, `out_w = W // sx`, `out_h = H // sy`. Sample source cell `(ry*sy, rx*sx)`, row-major, outer loop over rows.

**Scaling** [R]: samples where `u` and `v` are both finite define `u_min, u_max, v_min, v_max` (pairs only). Spans clamped to at least `1e-6`. Each channel: `q = round_ties_even((x - min) / span * 254 - 127)` for finite x, else `-128`. Decoding in client (`web/src/wind.ts`): `u = u_min + ((n + 127) / 254) * (u_max - u_min)`. [V: NaN -> -128; client formula matches]

**Bounds reported** [V: `test_wind_field_sends_nan_as_fill_and_reports_sampled_bounds`]: `lat_max = meta.lat_max`, `lat_min = meta.lat_max - (out_h - 1) * sy * lat_step` with `lat_step = (meta.lat_max - meta.lat_min) / max(1, H-1)`, `lon_min = meta.lon_min`, `lon_max = meta.lon_min + (out_w - 1) * sx * lon_step` with `lon_step = (meta.lon_max - meta.lon_min) / max(1, W-1)`.

**Success body:**
```json
{"ok": true, "timestamp": "2026-09-29T05:00:00+00:00", "width": 250, "height": 125,
 "lat_min": 20.0, "lat_max": 50.0, "lon_min": -130.0, "lon_max": -60.0,
 "u_min": -3.1, "u_max": 9.8, "v_min": -7.2, "v_max": 6.0,
 "u": [-128, 12, ...], "v": [-128, -14, ...]}
```
- Keys: `ok, timestamp, width, height, lat_min, lat_max, lon_min, lon_max, u_min, u_max, v_min, v_max, u, v`. The module docstring says `umin`/`umax`; the code and client use `u_min`/`u_max`. [V]
- `u` and `v` are flat arrays of length `width*height`, row 0 north.
- `timestamp` is the resolved (not requested) timestamp.
- Assumes `v` grid has the same dimensions as `u`; a mismatch can raise `struct.error` (uncaught, 500). [I]

**Headers:** success `public, max-age=300`; all failures no Cache-Control. [V]

### 5.7 `GET /api/lightning` (sync)
- Reads `STATE_DIR/lightning.json` and returns it parsed then re-encoded (passthrough). Written by `ingest-lightning` every ~2 s with a rolling 15-minute buffer (not validated here).
- Missing file: `200 {"type":"FeatureCollection","features":[],"generated_at":0,"reason":"no_data"}`, CC 10.
- Read or JSON error (`OSError`, `JSONDecodeError`): `200 {"type":"FeatureCollection","features":[],"reason":"state_read_failed"}`, CC 10 (note: no `generated_at`).
- Success CC 10. Any JSON value is returned as-is (no shape check). [R]

### 5.8 `GET /api/storms` (sync)
- Same pattern as lightning, file `STATE_DIR/storms.json` (written by ingest-mrms; see 13.5), CC 30.
- Missing: `{"type":"FeatureCollection","features":[],"reason":"no_data"}`. Error: `{"type":"FeatureCollection","features":[],"reason":"state_read_failed"}`. [V for missing]
- Success passthrough. This same file feeds storm-prefetch (5.9) and the briefing's incoming-storm calculation (7.2).

### 5.9 `GET /api/storm-prefetch` (sync)
**Query:** `lat` (float, required), `lon` (float, required), `zoom` (int, default 6), `palette` (str, default `classic`). Validation, in order, all 422 `{"detail": "..."}`:
- `lat`/`lon` out of range: `lat/lon out of range`.
- `zoom` not in `4..8`: `zoom must be between 4 and 8`.
- `palette` not in `{classic, vivid, muted}`: `unsupported palette`.

**Behavior** [R][V]:
1. Read `STATE_DIR/storms.json`; on `OSError` or `JSONDecodeError` use `{"type":"FeatureCollection","features":[]}`. (Not `read_manifest_file`-style last-good.)
2. `build_storm_prefetch_plan(...)` (section 5.9.1).

**No storm selected (or no `timestamp` in storms.json):**
```json
{"plan_id": null, "storm_cell_id": null, "bboxes": [], "tile_urls": []}
```
Four keys only; `generated_at` and `tracking_vector` are absent in this branch. [V]

**Storm selected:**
```json
{"plan_id": "2026-10-13T12:00:00+00:00:7:classic:6", "storm_cell_id": 7, "generated_at": 1760356800,
 "tracking_vector": {"east_kmh": 40, "north_kmh": 0, "speed_kmh": 40, "bearing_deg": 90},
 "bboxes": [
   {"lead_minutes": 0, "bbox": [-86.5, 42.5, -84.5, 43.5], "layer": "radar",
    "timestamp": "2026-10-13T12:00:00+00:00", "zoom": 6,
    "style_url": "https://<host>/api/storm-prefetch/style.json?layer=radar&palette=classic&timestamp=2026-10-13T12%3A00%3A00%2B00%3A00&zoom=6",
    "tile_urls": ["https://<host>/tiles/radar/classic/2026-10-13T12:00:00+00:00/6/16/23.png"]}],
 "tile_urls": ["https://<host>/tiles/radar/classic/..."]}
```
- `bboxes` has at most 3 entries (first 3 predictions of the selected storm). Entry keys: `lead_minutes, bbox, layer, timestamp, zoom, style_url, tile_urls`.
- If no frame matches a lead: entry has `layer: null, timestamp: null, zoom: <requested>, style_url: null, tile_urls: []`.
- `tile_urls` at top level is the de-duplicated (order-preserving) union of per-bbox URLs.
- `base_url` is `request.base_url` (scheme and host from the request, normally `https://<Host>` via uvicorn proxy headers) with no trailing slash.

**Storm selection** (`_select_storm`) [R]:
- For each feature: `bboxes = properties.predicted_bboxes`; for each prediction `bbox = [west, south, east, north]`, center `((south+north)/2, (west+east)/2)`; distance is haversine (`R = 6371.0088` km) from the request point to the center; take minimum over predictions.
- `severity = max(0, properties.peak_dbz (default 40) - 40)`; `area = max(1, properties.area_km2 (default 1))`; `score = distance - min(35, severity*0.8 + log10(area)*3)`.
- Features that raise KeyError/TypeError/ValueError while scoring are skipped.
- Lowest score wins (stable sort: first wins ties). Then reject if haversine from the request point to the feature's point geometry `geometry.coordinates` (lon, lat) exceeds `STORM_PREFETCH_MAX_DISTANCE_KM` (default 500). That check is outside the try block: a selected feature with no geometry raises (500). [R]

**Frame selection per lead** (`_frame_for_lead`) [R]:
- Lead 0: nearest `layer.radar.timestamps` entry to the anchor that is not after the anchor (`prefer_past`). No distance limit.
- Lead > 0: target = anchor + lead minutes; nearest `layer.nowcast.timestamps` entry; reject if more than 8 minutes (480 s) from target.
- Nearest-timestamp tie-break: the first candidate in list order (Python `min`).
- Anchor = parsed `storms.timestamp`; unparseable anchor returns the empty plan.

**Zoom:** `requested = max(4, zoom)` (already validated). `frame_zoom = min(requested, MAX_ZOOM[layer])` with `MAX_ZOOM = {radar: 7, nowcast: 6}`. Tile coordinates: Web Mercator tile for NW corner `(west, north)` (x_min, y_min) and SE corner `(east, south)` (x_max, y_max), inclusive ranges, both clamped to `[0, 2^z - 1]`, latitude clipped to `±85.05112878`. Only URLs whose file exists under `TILE_DIR` (`is_file()`) are included.

**Tile URL path:** `/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png`, each path component quoted with safe set `:+-.T_`. Disk path: `TILE_DIR / "{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png"`.

**style_url query:** `urlencode({"layer", "palette", "timestamp", "zoom"})` in that key order (`quote_plus`, so `:` becomes `%3A` and `+` becomes `%2B`). [V]

**plan_id:** `f"{storms.timestamp}:{cell_id}:{palette}:{requested_zoom}"`.

**Headers:** `Cache-Control: public, max-age=30`.

**Known error path:** a corrupt `manifest.json` raises out of `read_manifest_file` as ValueError (not caught here), so the response is 500, unlike `/api/manifest.json`. [I]

#### 5.9.1 `GET /api/storm-prefetch/style.json` (sync)
**Query:** `layer` (required), `palette` (required), `timestamp` (required), `zoom` (required int).
- `layer` not in `{radar, nowcast}` or `palette` not in the set: 422 `unsupported layer or palette`.
- `zoom` outside `4..max_zoom` (radar 7, nowcast 6): 422 `zoom outside layer coverage`.
- `timestamp` containing characters outside alnum and `:-_+.T`: 422 `invalid timestamp`.

**Success (CC 30):**
```json
{"version": 8, "name": "radar-ng storm prefetch",
 "sources": {"storm-prefetch": {"type": "raster",
   "tiles": ["<base_url>/tiles/nowcast/vivid/2026-10-13T12:05:00+00:00/{z}/{x}/{y}.png"],
   "tileSize": 256, "minzoom": 5, "maxzoom": 5}},
 "layers": [{"id": "storm-prefetch", "type": "raster", "source": "storm-prefetch", "paint": {"raster-fade-duration": 0}}]}
```
[V] `base_url` is `request.base_url` without trailing slash. `minzoom` and `maxzoom` both equal the requested zoom.

### 5.10 `GET /api/tropical` (sync)
- Same pattern as storms, file `STATE_DIR/tropical.json` (written by ingest-tropical from the NHC feed), CC 180.
- Missing: `{"type":"FeatureCollection","features":[],"reason":"no_data"}`. Error: `reason: state_read_failed`. [V for missing]

### 5.11 `GET /api/livez` (async)
- Returns `{"status":"ok"}`, 200, no Cache-Control. Never touches disk. Exempt from rate limiting (section 4.2) but still counted in metrics. [V: 5 of 5 succeeded past the limit]
- Caddy skips access logs for it. This is the k8s liveness and readiness target (through Caddy, so it also proves Caddy is alive, and it returns 502 when uvicorn is down). [R]

### 5.12 `GET /api/health` (sync)
**Exact logic** [R][V]:
```
now = time.time()
manifest = _build_manifest()              # 503 HTTPException if unreadable and no last-good
status = "ok"; reasons = []
mrms_age = age_seconds(manifest.layers.radar.latest, now)   # int, max(0, ...); None if missing/unparseable
if mrms_age is None: status = "degraded"; reasons += ["no_mrms_tiles"]
elif mrms_age > MRMS_MAX_AGE_S: status = "degraded"; reasons += [f"mrms_stale_{mrms_age}s"]
ns = read STATE_DIR/nowcast-status.json if it parses to an object, else None
if ns and ns.status == "degraded": status = "degraded"; reasons += [str(ns.reason or "nowcast_degraded")]
tiles_disk = statvfs(TILE_DIR) -> {total_bytes, used_bytes = total - free(f_bfree), percent = round(used/total*100, 1) or 0.0 if total == 0}; None on OSError (key still present)
skill = nowcast_skill summary (cached)
body = {...}
HTTP 200 if status == "ok" else 503
```
**Body:**
```json
{"status":"ok","mrms_age_s":0,"mrms_max_age_s":600,
 "nowcast":{"status":"ok","reason":null,...} or {"status":"unknown","reason":"no_status"},
 "nowcast_skill":{"available":false,"runs_scored":0,"headline":null,"reason":"no_scored_runs"},
 "reasons":[],
 "tiles_disk":{"total_bytes":270553174016,"used_bytes":241492578304,"percent":89.3},
 "upstream_forecast":"https://api.open-meteo.com/v1/forecast",
 "checked_at":"2026-10-13T12:34:56.123456+00:00"}
```
- `nowcast` is the raw `nowcast-status.json` object when readable, else `{"status":"unknown","reason":"no_status"}`.
- `nowcast_skill` has exactly `available`, `runs_scored`, `headline`, `reason` (reason null when available).
- `upstream_forecast` echoes `OPEN_METEO_BASE` (leaks the upstream URL; keep for compatibility).
- Empty state directory: 503 with `reasons: ["no_mrms_tiles"]`, `mrms_age_s: null`. [V]
- Fresh radar: 200, `reasons: []`, `mrms_age_s: 0`. [V]
- Manifest unreadable without last-good: 503 with body `{"detail":"manifest unavailable"}` (HTTPException body, not the health body). [R]
- No Cache-Control. [V]

### 5.13 `GET /api/basemap/style/{name}` (sync)
**Name check** [R]:
```
if not name.isalnum() and name not in ("positron", "dark-matter"):
    if "/" in name or ".." in name or "\\" in name: 400 {"detail":"invalid style name"}
path = STYLE_DIR / (name + ".json")
if not exists(path): 404 {"detail":"style not found"}
```
Note that a name such as `a b` passes the check and fails at 404. A `%2F` in the path is decoded by the router before matching, so it cannot reach this handler as a single segment. [R]

**Origin rewrite:** `origin = (x-forwarded-proto or request scheme) + "://" + (x-forwarded-host or Host header or request netloc)`. Headers are not validated.
- Each `sources.*.tiles[i]` that is a string starting with `/` becomes `origin + value`.
- Top-level `glyphs` and `sprite` strings starting with `/` become `origin + value`.
- Everything else is untouched (including `url` fields and external URLs).
- Body is the whole style document re-encoded.

**Headers:** none. There is no Cache-Control and no ACAO on this route. [V: `None`]

**Bundled styles:** `STYLE_DIR` defaults to `/srv/basemap/styles` in the image (`COPY basemap/styles/`). The snapshot does not contain `backend/basemap/styles/` (only the Dockerfile, README and fetch-glyphs.sh), so the bundled JSON was not inspected. [V]

### 5.14 `GET /api/metrics` (sync)
- `text/plain; charset=utf-8`. Format details in section 10.2.
- If the manifest is unreadable with no last-good copy, returns 503 JSON, not text.

### 5.15 `GET /api/alerts` (async)
**Query:** `lat` and `lon` (floats, both required). Out of range: 422 `{"detail":"lat/lon out of range"}`. Missing or non-numeric: 422 array.

**Cache key:** `"<round(lat,3)>,<round(lon,3)>"` (Python repr; e.g. `42.963,-85.668`, or `42.0,-85.0`). Cache TTL `ALERTS_TTL_S` (60 s), max `ALERTS_CACHE_MAX_ENTRIES` (1024), LRU touch on hit, expired entries dropped on get.

**Upstream:** `GET NWS_POINT_ALERTS_URL` with `params={"point": "<key>"}`, headers `User-Agent: NWS_USER_AGENT`, `Accept: application/geo+json`, timeout `PLACES_UPSTREAM_TIMEOUT_S` (6 s). [V: `point` = `42.963,-85.668`; User-Agent exact]

**Success:** NWS body returned unchanged (passthrough). Must be a JSON object with a `features` list, else treated as failure. CC `public, max-age=60`. Not cached when failing.

**Failure:** any `httpx` error, non-2xx, non-JSON, non-object body, or missing `features` list gives **502** `{"error":"upstream_unavailable"}` with `Cache-Control: no-store`. The body never contains `features`. [V]

### 5.16 `GET /api/alerts/map` (async)
- Cache key `__map__` (shares the `_alerts_cache` store, TTL 60 s).
- Upstream: `GET NWS_POINT_ALERTS_URL` with `params={"status": "actual"}`, same headers, timeout 12 s (2x).
- Keeps only features where `geometry` is truthy (storm-based polygons; zone-only alerts dropped). Output:
```json
{"type":"FeatureCollection","features":[
  {"type":"Feature","geometry":{"type":"Polygon","coordinates":[...]},
   "properties":{"id":"urn:...","event":"Severe Thunderstorm Warning","severity":"Severe","urgency":"Immediate",
                 "headline":"...","sent":"...","expires":"...","ends":"...","areaDesc":"..."}}]}
```
- `properties` keeps only these nine keys (missing ones are `null`): `id, event, severity, urgency, headline, sent, expires, ends, areaDesc`. [V: `description` removed]
- Geometry passes through unchanged. Feature `id` is not emitted (only `properties.id`).
- CC `public, max-age=60`. Failure: 502 `upstream_unavailable` no-store. [V]

### 5.17 `GET /api/geocode` (async)
**Query:** `q` (str, required, `min_length=2`, `max_length=100`, checked on the raw value), `limit` (int, default 8, `1..20`).

**Behavior:**
- Neither `GEOCODER_URL` nor `PHOTON_URL` set: 503 `{"error":"geocoding_not_configured"}`, `no-store`, no upstream call. [V]
- `query = " ".join(q.split())` (Unicode whitespace split). Cache key `query.lower() + "|" + str(limit)`. TTL `GEOCODE_TTL_S` (86400 s), max `GEOCODE_CACHE_MAX_ENTRIES` (1024).
- Note: a query that collapses to fewer than 2 characters (for example two spaces) is still sent upstream. [R]

**Geocoder branch** (when `GEOCODER_URL` set; wins over Photon) [V]: `GET {GEOCODER_URL}/v1/places` with `q=<query>` and `limit=<limit>`, timeout 6 s. Rows come from `resp.json()["results"]` (missing key = empty list). Each row maps through `_place_from_geocoder` (section 8.2); unmappable rows are dropped. No dedupe, no further limit. A non-2xx, non-JSON body, or a JSON document that is not an object raises and gives 502 no-store. [V for mapping and 502]

**Photon branch** [V]: `GET {PHOTON_URL}/api` with `q=<query>`, `limit=<limit*3>`, `osm_tag=place`, `lang=en`. Features mapped with `_place_from_feature(reverse=false)` (settlements only, section 8.1). De-duplicated by `id` (first wins). Stops at `limit`. Non-2xx or non-JSON gives 502 no-store.

**Success:** `{"results": [SelectedPlace, ...]}`. CC `public, max-age=3600` (note: server cache is 24 h, header says 1 h). [V]

### 5.18 `GET /api/reverse-geocode` (async)
**Query:** `lat`, `lon` (floats, required).

**Order:** parse (422) -> config check (503 `geocoding_not_configured`, no-store) -> range check (422 `lat/lon out of range`). [V for 503 with no upstream call]

**Cache:** key `"<round(lat,3)>,<round(lon,3)>"`, TTL `GEOCODE_TTL_S` (86400 s), max 1024. Response CC `public, max-age=3600`.

**Geocoder branch:** `GET {GEOCODER_URL}/v1/reverse-place` with `lat=<round(lat,3)>`, `lon=<round(lon,3)>` (strings), timeout 6 s.
- 2xx: `_place_from_geocoder(body)` (may be null if unmappable). Then sets `latitude`/`longitude` of the result to `round(lat,3)`, `round(lon,3)` (the requested point, not the matched place).
- 404: `place = null` (a real answer: nothing within range, e.g. open ocean); cached.
- Other status: 502 `upstream_unavailable` no-store, not cached. [V: 500 gives 502]

**Photon branch:** `GET {PHOTON_URL}/reverse` with `lat`, `lon` (rounded 3 dp), `limit=1`, `lang=en`. `features[0]` mapped with `_place_from_feature(reverse=true)` (section 8.1); result coordinates overwritten with the requested rounded point. No features: `place = null`, cached.

**Success body:** `{"place": SelectedPlace | null}`. [V]

### 5.19 `GET /api/briefing` (async) and `GET /api/alerts/explain` (async)
Described in section 7. Summary of params:
- `/api/briefing`: `lat` float `ge=-90 le=90` (required), `lon` float `ge=-180 le=180` (required), `place` str `max_length=80` (default empty). FastAPI 422 array for violations. [V]
- `/api/alerts/explain`: `id` str `min_length=8 max_length=200` (required). FastAPI 422 for short ids. [V]

---

## 6. Workflow routes (`/v1`)

Source: `backend/api/api/routes_workflows.py`, `temporal_client.py`, `storm_watch_activities.py` (worker side), `temporal/workflows/watch_storm.py`. Only the mobile-facing API surface is in scope here; the worker is not.

### 6.1 Feature flag
- Mounted at import time only when `os.environ.get("DISABLE_WORKFLOW_ROUTES") != "1"`. Any value other than exactly `1` (including `0`, `true`, empty) keeps routes enabled. [R]
- Read once at import. Changing it requires a restart.

### 6.2 Authentication (bearer HMAC token)

Configuration: `SIGNING_KEY = WORKFLOW_AUTH_SIGNING_KEY` (fallback `WORKFLOW_API_SIGNING_KEY`), read at import, encoded as UTF-8 bytes. Empty means auth is not configured.

Algorithm (exact) [R][V]:
```
if SIGNING_KEY is empty:                      -> 503 {"detail":"workflow API authentication is not configured"}
h = Authorization header (first value) or None
if h is None or not h.startswith("Bearer "):  -> 401 {"detail":"missing bearer token"}   (case-sensitive scheme)
token = h[len("Bearer "):].strip()                       (Python strip(): all Unicode whitespace)
payload_part, signature_part = token.split(".", 1)       (no "." -> ValueError -> 401 invalid)
expected = HMAC_SHA256(key = SIGNING_KEY, msg = payload_part as UTF-8 text)   # over the base64url TEXT
sig = b64url_decode(signature_part)
if not constant_time_equal(expected, sig):    -> 401 {"detail":"invalid bearer token"}
payload = json.loads(b64url_decode(payload_part))         (must be an object; TypeError -> 401)
subject = str(payload["sub"])                              (KeyError -> 401; any JSON type coerced by str())
expires_at = float(payload["exp"])                         (string or number accepted)
if subject == "" or expires_at <= time.time():             -> 401 invalid
return subject
```

`b64url_decode(s)`: append `"=" * ((-len(s)) % 4)`, then Python `base64.urlsafe_b64decode` (translates `-`/`_` to `+`/`/`, then non-validating decode that silently discards characters outside the alphabet, then errors via `binascii.Error` on bad padding). [R]

Verified behavior of edge cases [V]:
- `Bearer` scheme is case-sensitive: `bearer <tok>` gives 401 `missing bearer token`.
- Extra whitespace after `Bearer ` is accepted (strip).
- Padding `==` appended to the signature is **accepted** (padding is recomputed, so the decoded bytes are unchanged). Canonical encoding is not enforced.
- A trailing junk character on the signature gives 401 (it changes the length-based padding, so decoding differs). Not a general malleability proof; a Rust port should decode strictly (reject non-alphabet characters and non-canonical padding). That rejects nothing a valid issuer produces.
- `exp` as a string such as `"1789999999"` is accepted.
- `sub` as a JSON number `42` gives subject `"42"` (matches `user_id` `"42"` in paths).
- Expired, empty `sub`, missing dot, three segments, wrong key: all 401 invalid.
- Unset key: 503 even with a well-formed token. [V]

Token structure (issuer contract, from tests [V]):
`base64url_nopad(json_bytes) + "." + base64url_nopad(HMAC-SHA256(key, base64url_nopad(json_bytes)))` where `json_bytes` is compact JSON such as `{"sub":"user-42","exp":1789999999.5}`. Only `sub` and `exp` are checked. There is no `iat`, `aud`, `iss`, `nbf`, key id, rotation, revocation list or maximum lifetime.

Owner check: `_require_owner(authenticated, requested)` uses constant-time string compare; mismatch returns **404** `{"detail":"not found"}` (does not reveal existence). [V]

### 6.3 Endpoints

**POST `/v1/push-tokens`** (async)
- Body (Pydantic, extra keys ignored): `{"user_id": str 1..128, "token": str 16..512, "platform": "ios" | "android"}`. Violations give 422 array. [V: `platform: "web"` gives `literal_error`]
- `user_id` must equal the authenticated subject, else 404.
- Persists with `asyncio.to_thread(upsert, user_id, token, platform)` into SQLite (section 6.5). Success **200** `{"registered": true}`. [V]
- Re-registering the same token overwrites `user_id` and `platform` (`created_at` is kept). [R][V: second call returned 200]

**DELETE `/v1/push-tokens/{token}`** (async)
- Deletes the row where `user_id = <subject> AND token = <token>` (`delete_for_user`). Always **204**, even when nothing matched. [V: unknown token gave 204]
- A token belonging to another user is not deleted, and the response is still 204. Only the subject's own rows are affected. [R]
- Response body empty, `Content-Type: application/json` (Starlette quirk). [V]

**POST `/v1/watches`** (async)
- Body: `{"storm_cell_id": str 1..128, "user_id": str 1..128, "lat": float [-90,90], "lng": float [-180,180]}`. Pydantic lax mode: `lat` given as the string `"42.5"` is accepted and coerced; NaN is rejected. [V]
- Owner check first (404), then `get_client()`, then start. Order matters: a body that fails validation returns 422 before any Temporal call. [V]
- Success **201** `{"workflow_id": "watch:<hex>", "run_id": "<uuid or empty>"}`.
- `WorkflowAlreadyStartedError` -> **409** `{"detail":"already watching this storm"}`.
- `RPCError` during start -> client reset, **503** `{"detail":"temporal unavailable"}`.
- Connect failure on the first call (Temporal unreachable) raises `RuntimeError` from `Client.connect`, which is outside the try block, so the response is **500** (not 503). [V: `RuntimeError: Failed client connect: ...`]
- Workflow id: `"watch:" + sha256_hex(user_id + "\0" + storm_cell_id)` (UTF-8). The worker docstring says `watch:{user_id}:{storm_cell_id}`; that is out of date. The hashed form is what is in Temporal. [R][V]

**DELETE `/v1/watches/{user_id}/{storm_cell_id}`** (async)
- Owner check (404). Gets handle for the same id, `signal("unpinSignal")` with no payload.
- Success **204**. `RPCError` whose message contains `not found` (lowercased) -> 204 (already gone). Other `RPCError` -> client reset, **503**.
- Completed watch: see defect D2 in section 16 (likely 503, not 204).

**GET `/v1/watches/{user_id}/{storm_cell_id}`** (async)
- Owner check (404). `query("getCurrentState")`. Success **200**:
```json
{"last_frame_ts": "2026-10-13T12:00:00+00:00", "last_change_kind": "intensifying", "last_notified_at": 1760356800.5, "poll_count": 42, "push_count": 1}
```
- `last_frame_ts`, `last_change_kind`, `last_notified_at` may be null. `poll_count`, `push_count` are ints (default 0).
- `RPCError` containing `not found` -> **410** `{"detail":"watch no longer exists"}`; other `RPCError` -> **503**.
- Only these five fields are returned, even though the workflow state also holds `user_id`, `storm_cell_id`, `lat`, `lng`, `last_max_dbz`.

### 6.4 Temporal interactions

Client: `Client.connect(TEMPORAL_ADDRESS, namespace=TEMPORAL_NAMESPACE)` (plaintext gRPC; no TLS settings in code). Process-wide singleton, lazily created. On an `RPCError` in any call, `reset_client()` is called and the next request reconnects. [R]

| Action | Temporal call | Type / name | Payload | Options |
|---|---|---|---|---|
| start watch | `start_workflow` | workflow type `WatchStormWorkflow` | one JSON object: `{"user_id": str, "storm_cell_id": str, "lat": float, "lng": float}` (keys exactly these) | `id = watch:<hex>`, `task_queue = TEMPORAL_ALERTS_TASK_QUEUE` (default `radar-ng-alerts`), `id_reuse_policy = ALLOW_DUPLICATE_FAILED_ONLY` |
| unpin | signal | `unpinSignal` | no arguments | on the same id |
| read state | query | `getCurrentState` | no arguments | returns `WatchStormState` JSON |

Worker-side names, not called by the API but relevant to payload compatibility:
- Signal `alertMatchSignal(alert_id: str)` is sent by the worker activity `signal_matching_storm_watches` (single JSON string argument).
- Activities on task queue `radar-ng-alerts`: `persist_push_token`, `delete_push_token`, `compare_radar_frames`, `detect_storm_change`, `fan_out_push_to_user`, `fetch_nws_active_alerts`, `mark_alerts_seen`, `signal_matching_storm_watches`.
- `WatchStormState` JSON keys: `user_id, storm_cell_id, lat, lng, last_frame_ts, last_max_dbz, last_change_kind, last_notified_at, poll_count, push_count`.
- Workflow poll interval 60 s, max 1000 polls per run, continue-as-new after 24 h.

Both Python and Rust Temporal clients must encode the start and query payloads as JSON with the default `json/plain` payload encoding. [I: verify the Python default; the Rust 1.1.0 client in `/home/claude/rust-port/vendor-src` exposes `Client::start_workflow` with typed input, so a dynamic/raw start by name may need a shim]

### 6.5 Push token storage (SQLite)
- File: `STATE_DIR/push_tokens.sqlite` (directory created if missing). Connect per call, `sqlite3.connect` default timeout (5 s), `executescript` schema, commit on exit.
- Schema (must match, the worker reads the same file):
```sql
CREATE TABLE IF NOT EXISTS push_tokens (
    user_id    TEXT NOT NULL,
    token      TEXT NOT NULL PRIMARY KEY,
    platform   TEXT NOT NULL,       -- 'ios' | 'android'
    created_at REAL NOT NULL        -- unix seconds
);
CREATE INDEX IF NOT EXISTS idx_user ON push_tokens(user_id);
```
- Upsert: `INSERT ... ON CONFLICT(token) DO UPDATE SET user_id=excluded.user_id, platform=excluded.platform` (created_at untouched).
- `list_for_user(user_id)` and `delete_by_token` are worker-only in practice.
- Journal mode default (not WAL). A Rust port must use the same file with a busy timeout of at least 5 s, or the worker and API will collide. [R]
- Contradiction to flag: the worker activity `delete_push_token` says the tile server mounts state read-only, but the API's POST and DELETE write this file directly. If state is read-only in some deployment, both fail with an uncaught `sqlite3.OperationalError` and return 500. [I]

### 6.6 Error map summary (Python, for parity)
| Condition | Status | Body |
|---|---|---|
| SIGNING_KEY empty | 503 | `{"detail":"workflow API authentication is not configured"}` |
| missing/non-Bearer header | 401 | `{"detail":"missing bearer token"}` |
| bad token / sig / exp / sub | 401 | `{"detail":"invalid bearer token"}` |
| subject != requested user_id | 404 | `{"detail":"not found"}` |
| body validation | 422 | FastAPI array |
| already started | 409 | `{"detail":"already watching this storm"}` |
| watch gone (query) | 410 | `{"detail":"watch no longer exists"}` |
| RPCError | 503 | `{"detail":"temporal unavailable"}` |
| Temporal unreachable at first connect | 500 | Starlette `Internal Server Error` (defect D3) |
| sqlite error | 500 | Starlette `Internal Server Error` |

---

## 7. Briefing and alert explainer (LLM, optional)

Source: `backend/api/api/routes_briefing.py`. Decoration only: nothing else in the API depends on it, and every failure answers 200 `available:false`.

### 7.1 Configuration (all read at import)
`LLM_BASE_URL` (default empty, trailing `/` stripped), `LLM_API_KEY` (empty), `LLM_MODEL` (default `qwen3.8-27b`), `LLM_TIMEOUT_S` (20), `LLM_MAX_CONCURRENCY` (2), `BRIEFING_TTL_S` (600), `LLM_BREAKER_FAILURES` (3), `LLM_BREAKER_COOLDOWN_S` (300), `STATE_DIR`.

### 7.2 `GET /api/briefing`
**Flow** [R][V]:
1. `LLM_BASE_URL` empty: 200 `{"available":false,"reason":"not_configured"}` (no-store).
2. Breaker open (`allow()` false): 200 `{"available":false,"reason":"llm_unavailable"}` (no-store). Checked before the cache.
3. `place = _clean(place, 60)` where `_clean` collapses whitespace, strips, truncates.
4. Key: `f"{round(lat,1)},{round(lon,1)},{place.lower()}"`.
5. Cache hit (TTL `BRIEFING_TTL_S`, max 512 entries, insert-order eviction, hit does not refresh): 200 cached body, CC `public, max-age=120`.
6. In-flight: if the same key is being computed, await its future (shielded). If it yields a body: 200 that body (note: no Cache-Control on this path). Else 200 unavailable.
7. Otherwise become the leader and set a future for the key:
   - Gather concurrently, each best-effort (exceptions give defaults):
     - `forecast = get_forecast(lat, lon)` (section 5.2, rounds internally); default `{}`.
     - `nowcast = nowcast_point(lat, lon)` in a thread; default `{}`.
     - `storms = json.load(STATE_DIR/storms.json)`; default `{}`.
     - `alerts = get_alerts(lat, lon)` features' `properties` list; default `[]`.
   - If `forecast.current` is truthy: `facts = build_facts(...)`, `reply = _chat(BRIEFING_SYSTEM, json.dumps(facts), max_tokens=160)`. If `reply.body` is truthy: body = `{"available": true, "headline": _clean(reply.headline, 80), "body": _clean(reply.body, 400), "generated_at": <UTC iso>, "model": LLM_MODEL}`; cache it.
   - Future resolved with body or None in `finally`.
8. Body present: 200 with CC `public, max-age=120`. Absent: 200 `{"available":false,"reason":"llm_unavailable"}` no-store.

Known mislabel: forecast failure is reported as `llm_unavailable`, not a forecast reason. [V]

**Success body:**
```json
{"available": true, "headline": "Showers rolling in", "body": "Light rain by 2:40 PM, gone by 3:30.",
 "generated_at": "2026-10-13T12:34:56.123456+00:00", "model": "qwen3.8-27b"}
```

**Facts object** (`build_facts`, serialized with `json.dumps` default separators and `ensure_ascii=True` into the user message) [R]:
```json
{"place": "Grand Rapids" | "this spot",
 "local_time": "Monday 2:40 PM",
 "now": {"temp_f": 61, "feels_like_f": 60, "sky": "light rain", "wind_mph": 8, "gusts_mph": 15},
 "next_hour_radar": "rain from about 2:10 PM, peaking near 2:15 PM at 38 dBZ, ending around 2:20 PM" | "dry for the next hour" | "radar nowcast unavailable",
 "incoming_storm": {"minutes": 35, "peak_dbz": 57, "speed_mph": 37} | null,
 "next_12_hours": [{"time": "3 PM", "temp_f": 60, "sky": "light rain", "rain_in": 0.02}],   // up to 4 entries, every 3rd hour
 "today": {"high_f": 66, "low_f": 52, "rain_total_in": 0.3},
 "active_alerts": [{"event": "Flood Warning", "until": "2026-10-13T18:00:00-04:00"}]}     // max 3
```
- `local_time` uses `strftime("%A %-I:%M %p")` on `now_utc + utc_offset_seconds`.
- `next_12_hours`: start index is the first hourly time `>=` local now formatted `YYYY-MM-DDTHH:00` (string compare), else 0; take indices `start, start+3, start+6, start+9` (bounded by array length). `time` is `%-I %p` of the hourly ISO time. Temperatures and other values via `_r` (ties-even round) or raw.
- `sky` is a WMO code label (table in `routes_briefing.py`, e.g. 61 -> `light rain`, 0 -> `clear`). Unknown code -> `unknown`.
- `today.*` takes index 0 of the daily arrays.
- `active_alerts[].until` is `ends` or `expires`.

**Nowcast summary** (`summarize_nowcast`) [R][V]: wet = points with `dbz >= 15`. None: `"dry for the next hour"`. Else: `"rain from about <t(first)>"`, `"peaking near <t(peak)> at <round(peak.dbz)> dBZ"` (peak is the first maximum), and if the last wet point is not the final point: `"ending around <t(next point after last wet)>"`, joined by `", "`. `t(ts)` = time of `ts + offset` formatted `%-I:%M %p`.

**Incoming storm** (`incoming_storm`, radius 12 km) [R][V]: for each feature, `tracked = len(track_history) >= 3 and tracking_confidence >= 0.4`. For minutes `0, 5, ..., 60`: at minute 0 check the current point; later minutes only if tracked: extrapolate with `east_kmh`, `north_kmh` (`plon = clon + east*h / max(1e-6, 111.32*cos(clat))`, `plat = clat + north*h/110.574`, `h = minutes/60`), check haversine to the place (`R = 6371.0`) <= 12 km. Keep the earliest hit across features: `{"minutes", "peak_dbz": round(peak_dbz, default 0), "speed_mph": round(speed_kmh * 0.621, default 0)}`. [V: minutes 30-50, peak 57]

### 7.3 `_chat` (LLM call) [R][V]
- Non-blocking concurrency guard: `asyncio.Semaphore(LLM_MAX_CONCURRENCY)` created lazily; if locked, return None immediately (no queueing, breaker not updated).
- Breaker check first: `allow()` is `time.monotonic() >= open_until`. No half-open probe: after cooldown all calls are allowed again.
- Request: `POST {LLM_BASE_URL}/chat/completions`, headers `Authorization: Bearer <key>` only when key set, JSON:
```json
{"model": "<LLM_MODEL>",
 "messages": [{"role":"system","content":"<SYSTEM>"},{"role":"user","content":"<json facts or alert text>"}],
 "max_tokens": 160,            // briefing; 140 for explain
 "response_format": {"type":"json_object"},
 "temperature": 0.7, "top_p": 0.8, "top_k": 20, "min_p": 0.0,
 "presence_penalty": 1.5, "repetition_penalty": 1.0,
 "chat_template_kwargs": {"enable_thinking": false, "preserve_thinking": false}}
```
- Timeout `LLM_TIMEOUT_S` per request (uses the shared httpx client). No retries.
- Response: `choices[0].message.content or ""`, then `_parse_json`: remove `<think>...</think>` (DOTALL), take the substring from the first `{` to the last `}` (DOTALL, greedy), `json.loads`, must be an object.
- Exceptions (`httpx.HTTPError`, `ValueError`, `KeyError`, `IndexError`, `TypeError`): breaker failure, return None.
- Parsed None (including HTTP 200 with non-JSON content): breaker failure. Parsed object: breaker success (resets failure count).
- Breaker: consecutive failures counter; when it reaches `LLM_BREAKER_FAILURES`, `open_until = now + cooldown` and the counter resets to 0. [V: 5 requests with 503 upstream gave exactly 3 LLM calls, then `llm_unavailable`]

### 7.4 `GET /api/alerts/explain?id=`
**Flow** [R][V]:
1. Not configured: 200 `{"available":false,"reason":"not_configured"}` no-store.
2. Id must start with `urn:oid:` or `https://api.weather.gov/alerts/`, else 200 `{"available":false,"reason":"bad_id"}` no-store. [V]
3. Cache key `alert:<id>`, TTL `BRIEFING_TTL_S` (shared store with briefing, 512 entries). Hit: 200, CC `public, max-age=600`.
4. Fetch `GET https://api.weather.gov/alerts/<last path segment of id>`, headers `User-Agent: NWS_USER_AGENT`, `Accept: application/geo+json`, timeout 8 s. **Hard-coded host**, not `NWS_POINT_ALERTS_URL`. Error or non-JSON: `{"available":false,"reason":"alert_unavailable"}` no-store.
5. `properties` text: `{"event", "area": _clean(areaDesc, 300), "headline", "description": _clean(description, 1500), "instruction": _clean(instruction, 600), "ends": ends or expires}`.
6. `_chat(EXPLAIN_SYSTEM, json.dumps(text), 140)`. Missing `what`: `llm_unavailable` no-store.
7. Success: `{"available": true, "what": _clean(what, 300), "do": _clean(do, 300), "model": LLM_MODEL}`, CC `public, max-age=600`, cached.

For `id` with a `urn:oid:` prefix, the upstream path is `https://api.weather.gov/alerts/urn:oid:...` (no slash in the id). [R]

### 7.5 Verbatim prompts (must match)
`BRIEFING_SYSTEM` (concatenated Python string literals, shown as a single line):
```
You narrate the weather for a personal radar app. Be warm, a little playful, and concise. Use ONLY the JSON facts given; never invent numbers, alerts or storms. If there is an active alert or an incoming storm, say that first and plainly. Use local clock times like 2:40 PM. Reply as JSON: {"headline": "<= 8 words", "body": "1-2 short sentences, <= 45 words"}.
```
`EXPLAIN_SYSTEM`:
```
You explain official NWS weather alerts to a regular person. Use ONLY the alert text given. Reply as JSON: {"what": "one plain sentence on what is happening and where", "do": "one plain sentence on what to do"}. No hype, no emojis.
```

### 7.6 Caches and dedupe summary
- Briefing/explain cache: one `OrderedDict` keyed by string, value `(insert_time, body)`, TTL `BRIEFING_TTL_S`, cap 512, eviction in insertion order (hits do not refresh position). [R]
- In-flight map keyed by briefing key; only the leader calls the LLM. Explain has no in-flight dedupe. [R]

---

## 8. Places: geocoding and alerts shaping

Source: `backend/api/api/routes_places.py`. Client-side rounding is 2 decimals for city search results and 3 for points (`frontend/src/lib/coordinates.ts`).

### 8.1 Photon feature mapping (`_place_from_feature`)
Input: GeoJSON feature. `coords = geometry.coordinates`, need at least 2; `lon = coords[0]`, `lat = coords[1]`.
- `is_settlement = properties.osm_key == "place" and properties.osm_value in {city, town, village, hamlet, municipality, suburb, borough, quarter, neighbourhood, locality}`.
- Forward (`reverse=false`): requires `is_settlement`; `name = properties.name`.
- Reverse (`reverse=true`): `name = city or locality or district or (name if is_settlement) or county` (first truthy).
- Requires `name` truthy and `properties.osm_id` not null; `id = int(osm_id)` (a non-integer string raises ValueError, uncaught, 500: defect D8).
- Output: `{"id", "name", "latitude": lat, "longitude": lon, "admin1": state, "country": country, "countryCode": countrycode.upper() if str else null}`.

### 8.2 Geocoder row mapping (`_place_from_geocoder`)
Input row: `id` (int-able), `name` (str), `lat`, `lon` (float-able), optional `admin1`, `country`, `country_code`. Missing required key or bad type gives null (row dropped).
Output: `{"id": int, "name": str, "latitude": float, "longitude": float, "admin1": row.admin1 (raw), "country": row.country (raw), "countryCode": row.country_code (raw, NOT uppercased)}`.

[V: test row `country_code: "US"` -> `"US"`; unmappable row `{"bad":1}` dropped]

### 8.3 Frontend parse rule
`frontend/src/lib/geocoding.ts` `parsePlace` requires numeric `id`, string `name`, numeric `latitude`/`longitude`; `admin1`, `country`, `countryCode` are kept only if non-empty strings. Null values are therefore harmless.

### 8.4 Alerts shaping summary
See 5.15 and 5.16. Both are passthrough except `/alerts/map` filtering and slimming.

---

## 9. Grid readers (inspector, nowcast, wind)

### 9.1 Grid file format (writer: `backend/shared/grid_dump.py`)
- Metadata: `GRID_DIR/<layer>/<key>.meta.json`, JSON with keys `height` (int), `width` (int), `lat_min`, `lat_max`, `lon_min`, `lon_max` (float), `unit` (string), `fill` (float; `-9999.0` when the source had NaN), `stride` (int, downsampling factor), `data_file` (string, file name only).
- Data: `GRID_DIR/<layer>/<key>.<generation>.bin` where generation is 32 lowercase hex characters. Flat **little-endian float32**, row-major, `height` rows by `width` columns. **Row 0 is the north edge (lat_max); column 0 is the west edge (lon_min).**
- Commit pointer: meta is replaced atomically (tmp file then rename) after the data file is fsynced. Readers see a complete old or new generation. Old data files are removed by retention.
- Legacy layouts: if `data_file` is absent, the data file is `<key>.bin` (meta name with `.meta.json` replaced by `.bin`).
- Run-scoped nowcast grids: key `runs/<run_id>/<timestamp>`; a completed run has a marker file `.grid-generation-complete-v1.json` (not read by the API).

### 9.2 Bilinear sampling (`_sample_grid_point`) [R][V]
Inputs: `layer`, `timestamp`, `lat`, `lon`, optional `grid_key`.
```
meta_path = metadata_path(layer, timestamp, grid_key)     # sanitized, see 9.3
if meta_path missing: return {ok:false, reason:"grid_missing", layer, timestamp}
try: meta = json(meta_path); bin_path = binary_path(meta_path, meta)   (must be sibling; else ValueError)
     require bin_path exists (else OSError)
     h, w = int(meta.height), int(meta.width)
     lat_min, lat_max, lon_min, lon_max = floats
     unit = meta.unit or ""; fill = float(meta.fill or NaN)
except (OSError, KeyError, ValueError): return {ok:false, reason:"meta_invalid"}
if lat < lat_min or lat > lat_max or lon < lon_min or lon > lon_max:
    return {ok:false, reason:"out_of_bounds", lat, lon}
fx = (lon - lon_min) / (lon_max - lon_min) * (w - 1)
fy = (lat_max - lat) / (lat_max - lat_min) * (h - 1)
x0 = clamp(int(fx), 0, w - 2);  y0 = clamp(int(fy), 0, h - 2)   # int() truncates toward zero
dx = fx - x0;  dy = fy - y0
v00 = at(x0, y0); v10 = at(x0+1, y0); v01 = at(x0, y0+1); v11 = at(x0+1, y0+1)   # f32 little-endian, offset (iy*w+ix)*4
is_fill(v) = (v is NaN) or abs(v - fill) < 1e-6
valid = [v for v in (v00, v10, v01, v11) if not is_fill(v)]     # this order
if valid is empty: return {ok:true, value:null, unit, reason:"no_data"}
v0 = v00*(1-dx) + v10*dx   if neither v00 nor v10 is fill else valid[0]
v1 = v01*(1-dx) + v11*dx   if neither v01 nor v11 is fill else valid[-1]
value = v0*(1-dy) + v1*dy
return {ok:true, value: float(value), unit, layer, timestamp, lat, lon}
```
- Arithmetic is performed in f64 after widening each f32. Keep operation order exactly for bit-identical output (verified: `21.10000000000001`).
- Quirk to preserve: when one of the top corners is filled, `v0` is `valid[0]`, which can come from the bottom row, and `v1` uses `valid[-1]`, which can come from the top row. Verified: NW corner fill at (42.5, -85.5) gives `27.5`. [V]
- Second read error (`OSError` on open) gives `read_error`. A short read (`fh.read(4)` returns fewer than 4 bytes) raises `struct.error` (uncaught, 500). [R]
- Edge: `w == 1` or `h == 1` makes `w-2` negative; clamp then gives index -1 and reads the wrong cell. Not reachable with real grids. [R]

### 9.3 Path sanitization
- Layer (for paths): keep only alnum, `-`, `_`.
- Legacy timestamp key (inspector): keep only alnum and `:-_+.T` from the timestamp. Result becomes the file stem. Other characters are silently dropped (not rejected).
- Run-scoped key (nowcast): split on `/`; reject empty, `.`, `..` parts and any part with characters outside alnum and `:-_+.T` (ValueError -> `grid_missing`).
- Verified: the echoed `timestamp` in responses is the raw decoded path value, not the sanitized one. [V]

### 9.4 `/api/inspect` response contract
See 5.3. Always 200.

### 9.5 Nowcast point sampling
Uses 9.2 with `layer = "nowcast"`, `timestamp = frame.timestamp`, `grid_key = frame.grid_key` (run-scoped when present, else legacy). Each frame produces one point. Output shaping in 5.4.

### 9.6 Wind decode
Documented in 5.6.

---

## 10. Health and metrics

### 10.1 Health
See 5.12.

### 10.2 Metrics (`GET /api/metrics`) exact format
Output is `"\n".join(lines) + "\n"`, content type `text/plain; charset=utf-8`. Line order:

1. For each counter in this order, two lines: `# TYPE radar_ng_<name> counter` then `radar_ng_<name> <int>`:
   `forecast_requests_total`, `forecast_cache_hits_total`, `forecast_upstream_errors_total`, `manifest_requests_total`, `nowcast_point_requests_total`, `nowcast_point_cache_hits_total`, `rate_limited_requests_total`.
2. `# TYPE radar_ng_tile_timestamps gauge`, then one line per manifest layer in sorted name order: `radar_ng_tile_timestamps{layer="<name>"} <count>`, where count is `len(layer.frames)` if the key `frames` exists, else `len(layer.timestamps)`. Only dict layers.
3. If radar age is known: `# TYPE radar_ng_mrms_age_seconds gauge` then `radar_ng_mrms_age_seconds <int>`.
4. Nowcast skill: `# TYPE radar_ng_nowcast_skill_runs gauge`, `radar_ng_nowcast_skill_runs <runs_scored>`. If `by_lead` is non-empty, for `metric` in `pod`, `far`, `csi`: `# TYPE radar_ng_nowcast_<metric> gauge`, then for each lead row (ascending), for `threshold` in `rain`, `heavy`: if the value is not null, `radar_ng_nowcast_<metric>{lead_minutes="<L>",threshold="<T>"} <value>` (Python float repr, e.g. `0.8`).
5. `# TYPE radar_ng_http_requests_total counter`, then for each `(method, route, status)` key in sorted tuple order: `radar_ng_http_requests_total{method="GET",path="/api/livez",status="200"} <count>`.
6. `# TYPE radar_ng_http_request_duration_seconds_sum counter`, then for each `(method, route)` sorted: `radar_ng_http_request_duration_seconds_sum{method="GET",path="..."} <sum with 6 decimals>`.

[V: sample lines include `radar_ng_nowcast_skill_runs 8` and `radar_ng_nowcast_pod{lead_minutes="30",threshold="rain"} 0.8`]

Labels are not escaped. Route templates contain `{` and `}`, which Prometheus accepts inside quoted values.

Note: request counters are updated after the handler finishes, so a scrape does not include its own request.

---

## 11. In-process caches (complete list)

| Name | Holds | Key | TTL | Max entries | Eviction and notes |
|---|---|---|---|---|---|
| `_manifest_cache` | parsed manifest, encoded bytes, ETag | single entry | 15 s (wall clock) | 1 | refreshed on expiry; also used by nowcast_point and health |
| `_manifest_last_good` | last successful manifest body | single | none | 1 | used when re-read fails |
| `_skill_cache` | nowcast skill summary | single entry | 15 s | 1 | |
| `_forecast_cache` | Open-Meteo JSON | `"<lat1dp>,<lon1dp>"` | `FORECAST_TTL_S` (900) | `FORECAST_CACHE_MAX_ENTRIES` (512) | LRU on hit; expired popped |
| `_wind_cache` | wind-field response | resolved timestamp | 300 s (hard-coded) | `WIND_CACHE_MAX_ENTRIES` (48) | LRU; lock |
| `_nowcast_point_cache` | nowcast body without skill | `(run_id, lat3dp, lon3dp)` | none (run_id changes) | `NOWCAST_POINT_CACHE_MAX_ENTRIES` (1024) | LRU; lock |
| `_alerts_cache` | NWS point body or `__map__` | `"<lat3dp>,<lon3dp>"` or `__map__` | `ALERTS_TTL_S` (60) | `ALERTS_CACHE_MAX_ENTRIES` (1024) | LRU touch on get and put |
| `_geocode_cache` | `{"results": ...}` | `"<query lower>|<limit>"` | `GEOCODE_TTL_S` (86400) | `GEOCODE_CACHE_MAX_ENTRIES` (1024) | same |
| `_reverse_cache` | `{"place": ...}` | `"<lat3dp>,<lon3dp>"` | `GEOCODE_TTL_S` (86400) | 1024 | same; 404 result cached |
| `_cache` (briefing and explain) | briefing or explain body | `"<lat1dp>,<lon1dp>,<place lower>"` or `alert:<id>` | `BRIEFING_TTL_S` (600) | 512 | insertion order, hit does not refresh |
| `_inflight` | future per briefing key | key | duration of call | - | leader resolves |
| `_breaker` | failure count, open_until | single | cooldown 300 s | - | not half-open |
| `_semaphore` | LLM concurrency 2 | - | - | - | non-blocking acquire |
| `_rate_buckets` | token bucket per client | client key | none | 4096 | LRU |
| `_metrics`, `_request_counts`, `_request_duration_sums` | counters | fixed names / labels | none | unbounded by label set | counts since start |
| `_nowcast_point` `skill` value | merged per response | - | - | - | recomputed each response |

Caches are not shared across replicas or restarts. Several TTL caches are keyed with rounded coordinates so GPS jitter hits the same entry.

---

## 12. Upstream calls

| Purpose | Method and URL | Query or body | Headers | Timeout | Failure mapping |
|---|---|---|---|---|---|
| Forecast | GET `OPEN_METEO_BASE` | section 5.2 params | httpx default UA | 15 s (client default) | 502 `upstream_unavailable`, no CC; non-JSON 200 gives 500 (D5) |
| NWS point alerts | GET `NWS_POINT_ALERTS_URL` | `point=<lat3dp>,<lon3dp>` | `User-Agent: NWS_USER_AGENT`, `Accept: application/geo+json` | `PLACES_UPSTREAM_TIMEOUT_S` (6 s) | 502 no-store |
| NWS active map | GET `NWS_POINT_ALERTS_URL` | `status=actual` | same | 12 s | 502 no-store |
| Geocoder search | GET `GEOCODER_URL/v1/places` | `q`, `limit` | httpx default UA | 6 s | 502 no-store |
| Geocoder reverse | GET `GEOCODER_URL/v1/reverse-place` | `lat`, `lon` (3 dp) | httpx default UA | 6 s | 404 = null place; other = 502 no-store |
| Photon search | GET `PHOTON_URL/api` | `q`, `limit*3`, `osm_tag=place`, `lang=en` | httpx default UA | 6 s | 502 no-store |
| Photon reverse | GET `PHOTON_URL/reverse` | `lat`, `lon` (3 dp), `limit=1`, `lang=en` | httpx default UA | 6 s | 502 no-store |
| NWS alert detail | GET `https://api.weather.gov/alerts/<id-tail>` (hard-coded) | none | `User-Agent`, `Accept: application/geo+json` | 8 s | `alert_unavailable` |
| LLM | POST `LLM_BASE_URL/chat/completions` | section 7.3 JSON | `Authorization: Bearer` if key | `LLM_TIMEOUT_S` (20 s) | `available:false` |
| Temporal | gRPC to `TEMPORAL_ADDRESS` (namespace) | section 6.4 | - | client default | 503 on RPCError, 500 on connect failure |
| Disk | `STATE_DIR`, `GRID_DIR`, `TILE_DIR` (statvfs), `STYLE_DIR` | - | - | - | per endpoint |

Note: the forecast client is plain httpx with no `User-Agent`. Open-Meteo does not require one. NWS requires a contact in the `User-Agent`; the value must be set in production (docs/configuration.md).

Shared client: `app.state.forecast_http` (`httpx.AsyncClient`, pool 100/20, timeout 15 s) created in the lifespan and used by forecast, places, briefing and explain.

---

## 13. Shared on-disk formats the API reads

Produced by workers; the API only reads (except SQLite, section 6.5, and `manifest.lock`, not read here).

### 13.1 `manifest.json` (STATE_DIR)
Top-level: `schema_version` (2 when written), `layers` (object), `tile_url_template`, `tile_url_template_legacy`, `updated_at`. Layer: `timestamps`, `frames`, `latest`, `palettes`, plus layer metadata. Frontend validator `frontend/src/lib/manifest.ts` requires: ISO timestamps with offset, years 2000-2200, `timestamps` sorted and unique, `frames` length and timestamps identical in order to `timestamps`, `path` safe (no leading `/`, no `\ ? #`, no empty or dot segments), `palettes` string arrays, `lead_minutes >= 0`, `spatial_resolution_km > 0`, `max_zoom` integer 0..24, `kind` in `observation|nowcast|model_guidance`.

### 13.2 `nowcast-status.json` (STATE_DIR)
Object with `status` (`ok` or `degraded` read by health; `unavailable` used by nowcast branch), optional `reason`, optional `detail`. Keys beyond these are passed through in health's `nowcast` field.

### 13.3 `nowcast-skill.json` (STATE_DIR)
```json
{"schema_version":1,"window_hours":24,"thresholds_dbz":{"rain":20.0,"heavy":35.0},"updated_at":"...","entries":[
  {"anchor":"...","valid":"...","lead_minutes":30,"scored_at":"...","cells":10,
   "thresholds":{"rain":{"hits":8,"misses":2,"false_alarms":2},"heavy":{...}},
   "persistence":{"rain":{...},"heavy":{...}}}]}
```
Written compact with sorted keys. Unreadable or missing gives an empty log.

### 13.4 `lightning.json`, `tropical.json` (STATE_DIR)
GeoJSON FeatureCollections, passthrough. Tropical features are expected to have `kind` in `position|track|cone|forecast_point` with `category`, `name`, `storm_type` used by the web layer (`web/src/overlays.ts`).

### 13.5 `storms.json` (STATE_DIR)
Top-level: `type: "FeatureCollection"`, `timestamp` (ISO anchor), `generated_at` (number), `features`. Each feature: `geometry` Point `[lon, lat]`; `properties`: `cell_id`, `peak_dbz`, `area_km2`, `pixel_count`, `track_history` (list of `[t?, lon, lat]` triples; the client reads index 1 and 2), `tracking_vector` (`east_kmh`, `north_kmh`, `speed_kmh`, `bearing_deg`), `tracking_confidence`, `predicted_bboxes` (list of `{lead_minutes, bbox: [w, s, e, n]}`). Frontend reads `tracking_vector.speed_kmh` as mph after conversion. [R: `stormTracks.ts`]

### 13.6 Grid directories (GRID_DIR)
Section 9.1. Layers used by the API: `radar` (legacy keyed, inspector), `radar-hrrr`, `nowcast` (run-scoped), `wind_u`, `wind_v` (HRRR wind, timestamp keyed, `ok` decoded in 5.6).

### 13.7 Tiles (TILE_DIR)
`TILE_DIR/<layer>/<palette>/<timestamp>/<z>/<x>/<y>.png`. Only the existence of a file matters to the API (storm-prefetch). `disk_usage` on the directory for health.

### 13.8 Styles (STYLE_DIR)
`STYLE_DIR/<name>.json`. Passthrough with URL rewrite (5.13).

---

## 14. Environment variables (complete)

Read by the API process (names and defaults as in code). "Import" means read once at import, restart required.

| Variable | Default (code) | Read in | Notes |
|---|---|---|---|
| `TILE_DIR` | `/data/tiles` | server, health, storm prefetch | import |
| `GRID_DIR` | `/data/grids` | server (grid readers) | import; grid_dump has its own copy of this read |
| `STATE_DIR` | `/data/state` | server, routes_briefing, storm_watch_activities (not used in API), push_tokens, manifest, nowcast_skill | import |
| `OPEN_METEO_BASE` | `https://api.open-meteo.com/v1/forecast` | server | `.env.example` and docs say `http://open-meteo:8080/v1/forecast`; Dockerfile sets the public URL |
| `STYLE_DIR` | `/srv/basemap/styles` | server | import |
| `MRMS_MAX_AGE_S` | `600` | server (health) | |
| `FORECAST_TTL_S` | `900` | server | docs and `.env.example` say `300`; deployed value comes from configmap, verify |
| `FORECAST_CACHE_MAX_ENTRIES` | `512` | server | |
| `WIND_CACHE_MAX_ENTRIES` | `48` | server | |
| `NOWCAST_POINT_CACHE_MAX_ENTRIES` | `1024` | server | |
| `API_RATE_LIMIT_RPS` | `20` | server | float |
| `API_RATE_LIMIT_BURST` | `60` | server | float |
| `DISABLE_WORKFLOW_ROUTES` | unset (enabled) | server | only exact `1` disables |
| `WORKFLOW_AUTH_SIGNING_KEY` | `""` | routes_workflows | fallback `WORKFLOW_API_SIGNING_KEY`; import |
| `TEMPORAL_ALERTS_TASK_QUEUE` | `radar-ng-alerts` | routes_workflows | import |
| `TEMPORAL_ADDRESS` | `localhost:7233` | temporal_client | Dockerfile sets `temporal-frontend.temporal:7233`; docs say `temporal:7233` |
| `TEMPORAL_NAMESPACE` | `default` | temporal_client | |
| `LLM_BASE_URL` | `""` | routes_briefing | empty disables LLM; trailing `/` stripped |
| `LLM_API_KEY` | `""` | routes_briefing | |
| `LLM_MODEL` | `qwen3.8-27b` | routes_briefing | |
| `LLM_TIMEOUT_S` | `20` | routes_briefing | float |
| `LLM_MAX_CONCURRENCY` | `2` | routes_briefing | |
| `BRIEFING_TTL_S` | `600` | routes_briefing | also explain cache TTL |
| `LLM_BREAKER_FAILURES` | `3` | routes_briefing | |
| `LLM_BREAKER_COOLDOWN_S` | `300` | routes_briefing | |
| `NWS_POINT_ALERTS_URL` | `https://api.weather.gov/alerts/active` | routes_places | not in docs |
| `NWS_USER_AGENT` | `(radar-ng, admin@example.com)` | routes_places | placeholder; docs say `(radar-ng, you@example.com)`; storm_watch_activities has a different placeholder default. Must be set. |
| `ALERTS_TTL_S` | `60` | routes_places | |
| `ALERTS_CACHE_MAX_ENTRIES` | `1024` | routes_places | |
| `GEOCODER_URL` | `""` | routes_places | when set, wins over Photon |
| `PHOTON_URL` | `""` | routes_places | |
| `GEOCODE_TTL_S` | `86400` | routes_places | |
| `GEOCODE_CACHE_MAX_ENTRIES` | `1024` | routes_places | |
| `PLACES_UPSTREAM_TIMEOUT_S` | `6` | routes_places | float |
| `STORM_PREFETCH_MAX_DISTANCE_KM` | `500` | storm_prefetch | float; import |
| `NOWCAST_SKILL_WINDOW_HOURS` | `24` | nowcast_skill | import |
| `NOWCAST_SKILL_HEADLINE_LEAD_MIN` | `30` | nowcast_skill | import |
| `NOWCAST_SKILL_MIN_RUNS` | `6` | nowcast_skill | import |
| `GRID_MAX_AGE_S`, `GRID_MAX_CELLS`, `GRID_ORPHAN_GRACE_S`, `NOWCAST_INPUT_FRAMES`, `NOWCAST_INPUT_STEP_MIN`, `NOWCAST_GRID_INPUT_LAYER` | various | grid_dump (writer code, imported) | not used by read paths |
| `PORT`, `HOST` | - | not read; uvicorn flags in `start.sh` (`127.0.0.1:8000`, `--no-access-log`) | |
| `FORWARDED_ALLOW_IPS` | `127.0.0.1,::1` | uvicorn | affects `request.url.scheme` and `client` |
| `NWS_ALERTS_URL` | `https://api.weather.gov/alerts/active?status=actual` | storm_watch_activities (worker) | the API does not read it; docs list it as the worker poll URL |

Config conflicts to resolve before cutover: `FORECAST_TTL_S` (code 900, docs 300), `OPEN_METEO_BASE` (code and Dockerfile public, docs bundled), `TEMPORAL_ADDRESS` (code localhost, Dockerfile `temporal-frontend.temporal`, docs `temporal`), `NWS_USER_AGENT` (three different placeholders), and the privacy doc's claim about console-format logs.

---

## 15. Test invariant checklist

Every assertion in the API tests and the shared tests that constrain API inputs. Each item is a requirement for the Rust port. Test file in brackets.

### 15.1 Request controls and manifest [test_request_controls.py]
1. Client key uses `X-Client-IP` before XFF and peer. [client_key_prefers_caddy_client_ip]
2. Without `X-Client-IP`, client key is the rightmost XFF hop (`203.0.113.7`, not the first). [client_key_uses_last_forwarded_hop]
3. Client key falls back to peer host `10.0.0.9`. [client_key_falls_back_to_peer]
4. Route label is the matched template `/api/forecast/{lat}/{lon}`. [route_label_uses_template]
5. Unmatched routes label `unmatched`. [route_label_buckets_unmatched]
6. Manifest read failure serves last good copy; with none, 503. [build_manifest_serves_last_good_copy_then_503]
7. Manifest read and encoded once per TTL, shared with nowcast; `application/json`; `Cache-Control: public, max-age=15`. [manifest_is_read_and_encoded_once_per_ttl]
8. Manifest ETag is quoted, longer than 10 chars, `Cache-Control` present; `If-None-Match` equal gives 304 with empty body, ETag and Cache-Control; `W/` prefix matches; comma list matches; non-matching value gives 200. [manifest_carries_a_strong_etag_and_answers_304_on_match]
9. ETag changes when body changes; old ETag gives 200. [manifest_etag_changes_with_the_body]
10. Cached manifest without ETag gets one backfilled equal to blake2b of the encoded bytes. [cached_manifest_without_etag_backfills_one]
11. Skill endpoint: available with 8 runs; `headline.pod == 0.8`, `far == 0.2`, `lead_minutes == 30`, `runs == 8`. [nowcast_skill_endpoint_and_health_share_the_headline]
12. Health's `nowcast_skill.headline` equals the skill endpoint's headline. Same test.
13. Metrics text contains `radar_ng_nowcast_skill_runs 8` and `radar_ng_nowcast_pod{lead_minutes="30",threshold="rain"} 0.8`. Same test.
14. Nowcast point carries `skill.pod == 0.8`; cache-hit response carries the same skill object; with no skill log, `skill` key is present and null. [nowcast_point_carries_the_skill_headline]

### 15.2 Nowcast point [test_nowcast_point.py]
15. Complete MRMS series: `status ok`, `source mrms-nowcast`, `method` echoed, 2 points, first `dbz == 30.0`, `precipitation_mm_h > 0`, `spatial_resolution_km == 2.22`. [nowcast_point_samples_complete_mrms_series]
16. Warming grids: exact body `{"status":"unavailable","reason":"grids_warming_up","issued_at":"2026-07-15T16:00:00+00:00","points":[]}`. [nowcast_point_reports_warming_grids]
17. Three overlapping run generations: before manifest swap the old run answers 12 points all with dbz 10; after swap and prune, the new run answers 12 points with dbz 30; pruned run directory is gone; retained run directory exists. [three_overlapping_runs_keep_old_manifest_readable_until_swap]

### 15.3 Concurrency rules [test_server_concurrency.py]
18. These are plain `def` (threadpool): `get_manifest, inspect_point, nowcast_point, nowcast_skill_summary, wind_field, lightning, storms, storm_prefetch, storm_prefetch_style, tropical, health, get_basemap_style, metrics`.
19. These are `async def`: `get_forecast, livez`.
20. `livez` source contains none of `Path(`, `open(`, `read_text`, `read_bytes`, `iterdir`, `glob(`.

### 15.4 Wind field [test_wind_field.py]
21. NaN samples encode as `-128` in `u` and `v`; `u_min/u_max/v_min/v_max` finite; `lon_min == -130.0`, `lat_max == 50.0`; `lon_max` and `lat_min` match the stride formulas (pytest approx). [wind_field_sends_nan_as_fill_and_reports_sampled_bounds]
22. All-NaN grid: exact body `{"ok": false, "reason": "grid_empty"}`. [all_nan_wind_grid_is_reported_empty]

### 15.5 Places [test_routes_places.py]
23. Alerts proxy rounds to 3 dp (`point == "42.963,-85.668"`) and returns the NWS body unchanged; User-Agent is `NWS_USER_AGENT`. [alerts_proxy_rounds_point_and_passes_nws_body_through]
24. Two nearby points within the same 3-dp key make one upstream call. [alerts_are_cached_per_rounded_point]
25. Alert upstream 503 gives 502 with `cache-control: no-store` and no `features` key. [alert_upstream_failure_is_502_never_an_empty_list]
26. Failed alert fetch is not cached: next request calls upstream again and returns 200. [alert_failure_is_not_cached]
27. Photon search keeps settlements only (`city`, `town`), drops `house`, shapes to the app keys with `countryCode` uppercased, uses query path `/api`, `q` normalized (`grand  rap` becomes `grand rap`), `osm_tag=place`. [geocode_keeps_settlements_in_app_shape]
28. Reverse geocode through Photon returns the enclosing city name (`city` property) at the requested rounded point (`42.963`), upstream `lat=42.963`. [reverse_geocode_names_the_enclosing_city_at_the_requested_point]
29. Reverse geocode with no features: `{"place": null}`. [reverse_geocode_with_no_match_returns_null_place]
30. With `GEOCODER_URL` set, geocoder wins: path `/v1/places`, `q` normalized, `limit` passed (`5`), malformed row dropped, shape kept. [geocode_prefers_our_geocoder_and_keeps_app_shape]
31. Geocoder reverse: path `/v1/reverse-place`, `lat=42.963`, name and admin1 kept, latitude `42.963`; 404 gives `{"place": null}`; 500 gives 502. [reverse_geocode_via_our_geocoder_labels_the_requested_point]
32. No Photon and no geocoder: 503 for both routes, no upstream calls. [geocoding_without_photon_is_503_and_makes_no_request]
33. Out-of-range point (`lat=91`) on alerts: 422, no upstream call. [out_of_range_point_is_rejected]
34. Basemap style rewrites relative glyphs, tiles to `http://radar.example/...` from Host. [basemap_style_serves_absolute_self_hosted_glyphs]
35. Basemap style honors `x-forwarded-proto: https` and `x-forwarded-host`: all rewritten URLs start with `https://radar-ng-api.vanillax.me/`. [basemap_style_keeps_https_from_forwarded_proto]
36. Bundled styles reference no external host. [bundled_styles_reference_no_external_hosts] **Vacuous in this snapshot** (no `backend/basemap/styles/` directory; `glob` matches nothing). See D11.
37. Map alerts keep only polygon features, slim `properties` (no `description`), send `status=actual`, cache the second call (one upstream call). [map_alerts_keep_only_polygons_and_slim_properties]
38. Map alerts upstream 503 gives 502. [map_alert_upstream_failure_is_502]

### 15.6 Storm watch helpers [test_storm_watch_activities.py] (worker code, but the API imports the module for tests)
39. Point in polygon with hole: inside outer and outside hole true; outside outer false. [point_in_geojson_polygon_with_hole]
40. MultiPolygon membership. [point_in_geojson_multipolygon]
41. Alert id: `properties.id` wins over feature `id`; geometry kept. [alert_from_feature_keeps_id_and_geometry]
42. Mark seen: unhandled ids stay out of the seen file so the next poll retries. [mark_alerts_seen_skips_unhandled_so_next_poll_retries]
43. Mark seen: expired ids remain as cap filler (`{"active-1","expired-1"}`). [mark_alerts_seen_retains_expired_ids_as_cap_filler]
44. Mark seen without snapshot never forgets (`{"old-1","new-ok"}`). [mark_alerts_seen_without_snapshot_never_forgets]

### 15.7 Briefing and explainer [test_routes_briefing.py]
45. Unconfigured: exact body `{"available": false, "reason": "not_configured"}`. [unconfigured_llm_is_quietly_unavailable]
46. Briefing narrates facts with `chat_template_kwargs == {"enable_thinking": false, "preserve_thinking": false}`, user message facts include `place == "Grand Rapids"` and `now.sky == "light rain"`; second call at 1-dp-equal coordinates within TTL makes no LLM call. [briefing_narrates_facts_with_thinking_off_and_caches]
47. LLM 500, non-JSON content, and read timeout: all 200 `available: false`. [llm_failures_never_error_the_api] (parametrized over three)
48. Breaker stops calls after `LLM_BREAKER_FAILURES` consecutive failures; five requests gave exactly 3 LLM calls; the sixth reason is `llm_unavailable`. [breaker_stops_calling_a_failing_llm]
49. Forecast 502 gives `available: false`, 200. [missing_forecast_means_no_briefing_not_an_error]
50. Alert explainer: `urn:oid:` id gives `do == "Avoid low roads."`; `javascript:alert(1)` gives unavailable (`bad_id`). [alert_explainer]
51. `summarize_nowcast` output exact: `"rain from about 2:10 PM, peaking near 2:15 PM at 38 dBZ, ending around 2:20 PM"` for offset -14400; all-dry gives `"dry for the next hour"`. [nowcast_summary_and_incoming_storm_helpers]
52. `incoming_storm` for a storm 0.5 degrees away moving east at 60 km/h returns minutes in [30, 50] and `peak_dbz == 57`. Same test.

### 15.8 Workflow auth [test_routes_workflows_auth.py]
53. Valid scoped unexpired token returns subject `user-42`. [authenticated_user_accepts_scoped_unexpired_token]
54. Malformed (`Bearer not-base64.not-base64`), `Basic credentials`, and missing header each give 401. [authenticated_user_rejects_malformed_credentials]

### 15.9 Shared invariants that constrain API inputs
55. Nowcast skill: `scores_from_counts` classic table; append keeps only window entries and writes atomically; unreadable log is empty; headline withheld below `MIN_RUNS`; headline is the 30-minute bucket; no-rain gives `no_rain_observed`; Prometheus lines expose each lead and threshold. [backend/shared/test_nowcast_skill.py: 7 tests]
56. Manifest: update adds and removes timestamps; replace swaps all timestamps; empty list drops the layer; missing file reads as empty; corrupt file raises instead of wiping; unreadable raises. [backend/shared/test_manifest.py: 6 tests]
57. Grid metadata points atomically to a versioned binary; prune keeps newest N with their data files; orphans removed; in-flight tmp kept; keep-zero removes all committed generations; prune waits for metadata commit. [backend/shared/test_grid_dump_atomic.py, test_grid_dump_prune.py]

### 15.10 Coverage gaps (no test exists)
- Rate limiting behavior (429, bucket math, refill) has no automated test. Verified manually here (section 4.2).
- Forecast: no test for cache key rounding, TTL, 502, or passthrough body. Verified manually here.
- Inspector: no test for bilinear values, fill behavior, out-of-bounds, or `meta_invalid`. Verified manually here.
- Health: no test for ok, degraded reasons, or statvfs fields. Verified manually here.
- Lightning, storms, tropical, storm-prefetch, style.json: no tests.
- Watch lifecycle (start, query, unpin, 409, 410, 503): no tests. Only auth is tested.
- Push tokens: no tests. Verified manually here.
- Metrics output format: only two lines asserted.
- CORS, caching headers on Caddy-served paths: no tests of any kind (Caddy not in CI).

---

## 16. Defects, divergences and port decisions

Each item says whether the Rust port should preserve the current behavior (P) or fix it (F). Items marked F are intentional divergences that the app does not depend on; confirm before shipping.

| ID | Behavior today | Evidence | Recommendation |
|---|---|---|---|
| D1 | Watch re-start after the workflow completed (after unpin) is blocked: `ALLOW_DUPLICATE_FAILED_ONLY` only allows reuse after failure/termination, so a completed watch returns 409 `already watching this storm`. | code [R]; policy semantics [I] | F: use `ALLOW_DUPLICATE` for closed workflows, or check the handle's status and restart. Verify against a live Temporal first. |
| D2 | `DELETE /v1/watches` on a completed workflow probably returns 503 (signal error text is "already completed", and the code only treats "not found" as gone). | code [R]; server message text [I] | F: treat "already completed" as success (204). |
| D3 | Temporal unreachable at first connect returns 500 (`RuntimeError` from `Client.connect`, outside try). | [V] | F: return 503 `temporal unavailable`. |
| D4 | Briefing forecast failure reports `llm_unavailable`. | [V] | F (cosmetic): report `forecast_unavailable`. |
| D5 | Forecast upstream 200 with non-JSON body gives 500. | [V] | F: 502 `upstream_unavailable`. |
| D6 | Any NaN or Infinity in a JSON body gives 500. | [V] | F: serialize NaN as `null` (serde_json does this for f64 NaN in `Value`; confirm). |
| D7 | Inspector with NaN lat or lon gives 500 (`int(nan)` ValueError). | [I] | F: return `out_of_bounds` or 422. |
| D8 | Photon `osm_id` non-integer string gives 500. | [I] | F: drop feature. |
| D9 | Storm prefetch with a corrupt `manifest.json` gives 500 (manifest endpoints serve last-good or 503). | [I] | F: reuse the manifest last-good path. |
| D10 | `/api/basemap/style/{name}` sends no Cache-Control and no ACAO. | [V] | P (or add `max-age=3600` if clients are fine; not required by the app). |
| D11 | Test `test_bundled_styles_reference_no_external_hosts` passes vacuously in this snapshot (`backend/basemap/styles/` absent). The Dockerfile `COPY basemap/styles/` would also fail to build from this tree. | [V] path missing | F: restore the styles directory or make the test fail when it is empty. |
| D12 | Caddy `precompressed gzip` on tiles is inert; no `encode` directive anywhere. | [R] | P (keep inert), or add compression for JSON only where the edge does not already compress. Avoid compressing PNG tiles and 304s. |
| D13 | Unmatched paths return Caddy 200 empty, not 404. | [R] | F: 404. |
| D14 | Explainer hard-codes `api.weather.gov` and ignores `NWS_POINT_ALERTS_URL`. | [R] | F: make it configurable (needed for tests). |
| D15 | Briefing and explain responses are `Cache-Control: public`, and the URL carries rounded coordinates. | [R] | P (privacy doc accepts ~1 km coords). Consider `private` for briefing. |
| D16 | Briefing in-flight waiters get 200 without Cache-Control. | [R] | F (cosmetic). |
| D17 | `incoming_storm` raises on a null `tracking_confidence` (outside `_best_effort`), so the whole briefing returns 500. | [I] | F: coerce null to 0. |
| D18 | Wind field: `v` grid dimensions are not checked against `u`; short data raises `struct.error` (500). | [I] | F: validate sizes, return `grid_read_failed`. |
| D19 | Inspector bilinear fallback quirk (`valid[0]`/`valid[-1]`). | [V] | P: keep exactly (output must match). |
| D20 | Health exposes `upstream_forecast` (the Open-Meteo URL). | [V] | P (compatibility). |
| D21 | Push token POST and DELETE write SQLite in the API process; a comment says state is read-only for the API. | [I] | Confirm the mount mode; if read-only, moving writes to the worker needs a queue. |
| D22 | `X-Client-IP` is trusted without checking the peer. Safe only because uvicorn binds loopback. | [R] | P: keep binding loopback; in Rust, bind loopback too, or resolve client IP from the peer. |
| D23 | Rate-limit `Retry-After` is always `1`. | [V] | P. |
| D24 | Pydantic lax coercion accepts `"42.5"` for `lat`. | [V] | P (harmless). |
| D25 | Basemap style origin trusts `X-Forwarded-Host` and `X-Forwarded-Proto` without validation. Only affects the caller's own response. | [R] | P. |
| D26 | Forecast cache `max-age` is the full TTL on hits, not the remaining TTL. | [R] | P (or decrement; clients do not depend on it). |
| D27 | Doc drift: `docs/privacy.md` (console logs), `docs/tuning.md` (`@observed` matcher, per-layer tile rules), `docs/configuration.md` (defaults above). | [R] | Trust code; fix docs. |
| D28 | Alert and place caches are shared across all points in a 3-dp cell; 60 s alert TTL is shorter than the 3-dp cell's lifetime. | [R] | P. |

---

## 17. Verification log

Environment: scratch venv with fastapi 0.143.0, starlette 1.8.0, httpx 0.28.1, uvicorn (latest satisfying >=0.32), temporalio 1.30.0, numpy, pytest. Backend copied from the read-only tree to scratch; no writes to the source.

- Full test run: `pytest backend/api/api backend/shared/test_nowcast_skill.py backend/shared/test_manifest.py` gives **68 passed** (1 Starlette deprecation warning about httpx). [V]
- Probe 1 (empty state and error bodies): 422 range, 422 parse, 404, 405 with `Allow: GET`, health 503 on empty state, metrics format, storm-prefetch empty shape, style zoom 422, basemap 404 with no CC, wind-field `grid_missing` with no CC, lightning/storms/tropical empty bodies, livez, nowcast no-frames branch (no `skill` key), skill empty body, briefing not_configured no-store. [V]
- Probe 1 also: Starlette JSON rendering is compact and UTF-8; NaN raises ValueError; Python `round(2.5) == 2`. [V]
- Probe 2 (rate limit and tokens): burst 3 with rps 0 gives 200,200,200,429,429; 429 body `{"error":"rate_limited"}`, `Retry-After: 1`; livez exempt. Token edge cases as in 6.2. Storm prefetch full output with style_url encoding and plan_id; `tiles` list empty because the probe wrote files to the wrong directory (not a server defect). [V]
- Probe 3 (grids and health): inspector bilinear 25.0 at cell center; NW corner exact 10.0; out-of-bounds shape; missing timestamp shape; fill quirk value `27.5`; second fill value `21.10000000000001`; health ok with fresh radar (`mrms_age_s: 0`, `reasons: []`); manifest ETag and 304; briefing with forecast 502 gives `llm_unavailable`; place longer than 80 gives 422; explain bad id gives `bad_id`. [V]
- Probe 4 (workflow routes, no Temporal): push token register 200, upsert 200, owner mismatch 404, bad platform 422 `literal_error`, no auth 401, delete 204 (including unknown token), watch owner mismatch 404 before any Temporal call, lat 95 422, GET/DELETE other-owner 404. [V]
- Probe 5: Temporal unreachable at connect gives `RuntimeError: Failed client connect`, not RPCError (D3); Pydantic lax `"42.5"` accepted; NaN rejected. [V]
- Probe 6 (forecast): closed upstream gives 502 without CC; cache key rounding `42.96 -> 43.0`; second request in the same cell is a hit (one upstream call); exact upstream query string; non-JSON 200 raises JSONDecodeError (D5); HEAD and OPTIONS give 405. [V]
- Source inspection of Caddyfile, start.sh, Dockerfile, docs, frontend and web clients (sections 2.1, 3, 13, 14). [R]
- uvicorn 0.x source: `proxy_headers=True`, `forwarded_allow_ips` default `127.0.0.1,::1` (env `FORWARDED_ALLOW_IPS`). [V]
- Temporal SDK 1.30.0: `WorkflowIdReusePolicy.ALLOW_DUPLICATE_FAILED_ONLY` exists; `WorkflowAlreadyStartedError` is a `FailureError`, not an `RPCError` (so the 409 branch is reached first). [V]
- Rust SDK vendored at `/home/claude/rust-port/vendor-src/temporalio-client-1.1.0`: `Client::start_workflow` is typed by `HasWorkflowDefinition`; `WorkflowIdReusePolicy::AllowDuplicateFailedOnly` exists. [V]

Not verified (needs a live system): Caddy 2.11.4 header behavior (X-Forwarded-Host, file_server ETag/Range, handle_errors header override ordering, precompressed on missing `.gz`); Temporal server error text for signal to a completed workflow (D2); Temporal reuse-policy behavior for completed runs (D1); the Python Temporal payload encoding name; the bundled style JSON contents.

Scratch artifacts: `probe/probe1.py` to `probe6.py` and the copied backend (in the session scratch directory, not in this repository).

---

## 18. Rust design notes

### 18.1 Suggested crates (verify versions in the workspace)
- HTTP: `axum` 0.8 (`{param}` path syntax), `tower-http` 0.6 (`ServeDir`, `TraceLayer`, optionally `CompressionLayer`, `TimeoutLayer`), `hyper` via axum.
- Upstream: `reqwest` 0.12 with rustls, one shared `Client` (pool 100 per host, keep-alive 20 idle), per-request timeouts as in section 12.
- JSON: `serde_json` (use `Value` for passthrough documents; enable `preserve_order` if key order must match Python exactly, which the app does not need).
- Caches: `moka` 0.12 for TTL+capacity stores (forecast, alerts, geocode, reverse, briefing, nowcast point, wind). Use `arc-swap` for the single-entry manifest and skill caches with an explicit `Instant` expiry.
- Rate limiter: a custom token bucket (section 4.2) in `Mutex<LruCache<String, (f64, Instant)>>` capped at 4096. `governor` is GCRA, with different burst and retry semantics; use it only if the exact algorithm is not required.
- Crypto and encoding: `hmac` + `sha2` (constant-time verify), `blake2` (`Blake2bVar` or `Blake2b<U12>` for ETag), a strict base64url decoder (section 6.2).
- SQLite: `rusqlite` with `bundled`, `busy_timeout(5s)`, same schema (section 6.5). Use `spawn_blocking`.
- Filesystem: `tokio::task::spawn_blocking` for every disk-touching handler. Bound concurrency with a `Semaphore` (Python used anyio's default threadpool of about 40). `/api/livez` never touches disk and runs on the runtime.
- Stats: `nix::sys::statvfs` (or `libc::statvfs`) for `tiles_disk`; use `blocks_free()` (`f_bfree`), not `f_bavail`, to match `shutil.disk_usage`.
- Temporal: the vendored `temporalio-client` 1.1.0 at `/home/claude/rust-port/vendor-src`. Starting by string name with a raw JSON payload may need a small dynamic-workflow shim or direct RPC use. Keep the payload JSON keys exactly as in 6.4.
- Logging: `tracing` with JSON `tracing-subscriber`. Field names for Loki: `method`, `uri`, `status`, `duration`, `user_agent`. Skip `/api/livez` and `/api/health`.

### 18.2 Module layout
```
radar_api/
  config.rs          // all env vars, parsed once at startup (section 14); reject bad values loudly
  error.rs           // ApiError -> status + body helpers for {"detail"}, {"error"}, text 500
  router.rs          // route table (section 2), middleware order
  mw/rate_limit.rs   // section 4 exactly
  mw/metrics.rs      // counters, duration, route template labels
  json.rs            // compact serializer helpers; NaN -> null policy
  util/pyround.rs    // py_round(x, n), round_ties_even helpers, Python-like float repr
  cache.rs           // moka wrappers + manifest/skill snapshot types
  state/manifest.rs  // read_manifest_file normalization, ETag, last-good
  state/files.rs     // lightning, storms, tropical, nowcast-status, nowcast-skill
  grid/read.rs       // meta + f32 bilinear sampling (section 9.2)
  grid/wind.rs       // decimation and int8 scaling (section 5.6)
  handlers/*.rs      // one module per route family
  upstream.rs        // reqwest client and per-upstream helpers
  places.rs          // Photon/geocoder mapping (section 8)
  briefing.rs        // facts, prompts, breaker, semaphore, single-flight
  workflows/auth.rs  // bearer verification (strict base64url)
  workflows/temporal.rs
  store/push_tokens.rs
```

### 18.3 Concurrency and state rules
- Single process, multi-thread runtime. The Python app effectively ran one event loop with a threadpool, so cache updates were not contended. In Rust, guard shared mutable state (rate limiter, breaker, single-flight map) with short critical sections; never hold a lock across `.await`.
- Manifest and skill: one snapshot each, replaced when expired; under contention, two readers may both refresh (same as Python, acceptable).
- Briefing single-flight: first request for a key becomes the leader; followers await a `watch` or `oneshot` result; leader failure gives `llm_unavailable` to all followers.
- `try_acquire` on the LLM semaphore; no queueing (matches `_semaphore.locked()` check).

### 18.4 Caddy: drop or keep
- Keep Caddy for Phase 1 (section 3.9). Its header semantics are the contract, and keeping it removes the need to reproduce `handle_errors` ordering and the imagery and PMTiles proxies on day one.
- Phase 2 (optional, after parity): serve `/tiles`, `/basemap/styles`, `/basemap/fonts` with `ServeDir` plus middleware for the CC, ACAO and error override; proxy `/basemap/tiles` to the go-pmtiles sidecar (keep the sidecar; a Rust PMTiles reader is extra scope); add a cache to the USGS imagery proxy (currently uncached).
- The trusted-proxy rule (strict right-to-left over XFF with private ranges) must be implemented in Rust if Caddy is removed. Algorithm: peer = socket address. If peer is not private, client = peer. Else walk XFF from the right, skip addresses that are private (or loopback), first public address is the client; if none, client = leftmost. Verify against Caddy 2.11.4 on a test with a fake XFF chain.

### 18.5 Parity testing
- Build a parity harness that starts the Python app (`uvicorn backend.api.api.server:app --host 127.0.0.1 --port 8000 --no-access-log`) and the Rust binary on the same fixture directories (`STATE_DIR`, `GRID_DIR`, `TILE_DIR`, `STYLE_DIR`) and the same stub upstreams (`OPEN_METEO_BASE`, `NWS_POINT_ALERTS_URL`, `PHOTON_URL`, `GEOCODER_URL`, `LLM_BASE_URL`; plus an override for the hard-coded `api.weather.gov` in the explainer, D14).
- Compare: status code; `Cache-Control`, `ETag`, `Retry-After`, `Content-Type`, `Allow` headers; JSON bodies semantically (key set, values with `1e-9` relative tolerance for floats, array order preserved); text for metrics after normalizing values that are time-dependent (`checked_at`, `updated_at`, `generated_at`, `mrms_age_s`, duration sums).
- Fixtures: reuse the Python test fixtures (section 15), plus the probe inputs in section 17.
- Replay the same sequence of requests against both and assert equal rate-limit responses (`burst` and `rps` set via env in both).
- Golden tests for pure functions: `summarize_nowcast`, `incoming_storm`, `build_facts`, `build_storm_prefetch_plan`, `_sample_grid_point`, `wind_field` decimation, `_point_in_geojson`, the bearer verifier (including the edge cases in 6.2 as expected outcomes under the strict decoder).

### 18.6 Things to leave alone
- Payload field names, enum strings, WMO sky labels, prompt text, sampler values, and the exact NWS `User-Agent` handling. Clients and the worker depend on them.
- Path and query parsing that accepts the URL-encoded `+` in timestamps (`%2B`) and the exact timestamp sanitization sets.
- The `Cache-Control` values and the 304 rule.

---

*End of spec.*
