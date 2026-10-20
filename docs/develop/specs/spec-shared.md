# spec-shared: behavioral spec for `backend/shared/` (radar-ng -> Rust)

Source of truth: `/mnt/user-data/uploads/radar-ng/backend/shared/` (read-only). Line references are to that tree.
Scope: `manifest.py`, `state.py`, `grid_dump.py`, `storms.py`, `storm_prefetch.py`, `storm_watch.py`,
`push_tokens.py`, `nowcast_skill.py`, `logger.py`, `activity_heartbeat.py`, and their tests
(`test_manifest.py`, `test_grid_dump_atomic.py`, `test_grid_dump_prune.py`, `test_storms.py`,
`test_nowcast_skill.py`, `test_activity_heartbeat.py`). `tiler.py`, `png_palette.py`, `palettes.py` and
`test_tiler*` / `test_png_palette` are out of scope.

Client contracts consulted: `frontend/src/lib/manifest.ts`, `frontend/src/types/weather.ts`, `web/src/frames.ts`.
Architecture: `ARCHITECTURE.md`.

Callers were checked so that the spec describes real usage: `ingest_mrms/activities.py`,
`ingest_hrrr/activities.py`, `ingest_airquality/activities.py`, `nowcast/activities.py`, `nowcast/skill.py`,
`tile_cleanup/activities.py`, `api/api/storm_watch_activities.py`.

Method: every in-scope source file was read in full, and the callers listed above were read for the parts that matter. **No Python was executed for this spec.** Defect and hazard findings (sections 6.5, 8.5, 14) come from reading the code. Confirm each one by running the Python before the port depends on it.

Conventions used below:
- **MUST / MUST NOT** mark behavior that a byte-compatible port has to reproduce.
- **[DEFECT]** marks behavior that looks unintended. Section 14 lists them for a decision. Do not silently "fix" or copy them; decide per item.
- **[HAZARD]** marks non-obvious behavior that a naive port would get wrong.
- Defaults shown as `ENV_VAR` (default) are read at module import unless noted.

---

## 0. Global conventions

### 0.1 Timestamps

- Canonical frame timestamp is `datetime.isoformat()` of a UTC-aware datetime, e.g. `2026-05-20T12:00:00+00:00`.
  Origin: `_extract_timestamp` in `ingest_mrms/activities.py:138`, and `latest_iso` values in `nowcast/activities.py`.
- Timestamps are used as **strings** for identity (manifest `timestamp`, grid file names, ProcessedSet keys).
- Manifest sorting is **lexicographic on the string** (`sorted()` on `str`), not chronological. This only equals chronological order if every writer uses the same UTC `+00:00` form. See 3.9.
- Parsing (`datetime.fromisoformat(s.replace("Z", "+00:00"))`) is used in `storms.py`, `storm_prefetch.py`, `nowcast_skill.py`, and `nowcast/skill.py`. The **naive-datetime policy differs by module**:
  - `storms.py` `_parse_timestamp` and `storm_prefetch.py` `_parse_timestamp`: naive stays naive. Mixing naive and aware in a subtraction raises `TypeError`, which is not caught (see 14).
  - `nowcast_skill.py` `_parse_ts` and `nowcast/skill.py` `_parse_ts`: naive is coerced to UTC (`dt.replace(tzinfo=timezone.utc)`).
  - Python's `fromisoformat` (3.11+) accepts many forms. A Rust port needs an explicit parser. RFC 3339 covers all forms the code writes.

### 0.2 JSON writing styles (exact)

Python `json.dump` defaults are `ensure_ascii=True`. So every non-ASCII character is written as `\uXXXX` (for example `"°"` becomes `"°"`, `"—"` becomes `"—"`, `"→"` becomes `"→"`). Non-finite floats are written as the bare tokens `NaN`, `Infinity`, `-Infinity` (not valid strict JSON).

| Writer | Separators | Keys | Trailing newline | Used by |
|---|---|---|---|---|
| A. compact sorted | `(",", ":")` | `sort_keys=True` | yes (`"\n"`) | `manifest.json`, grid `*.meta.json`, grid generation marker, `nowcast-skill.json` |
| B. compact unsorted | `(",", ":")` | insertion order | yes | `storms.json` |
| C. default | `(", ", ": ")` | insertion order | **no** | ProcessedSet state files (`{"items": [...]}`) |
| D. logger | `(", ", ": ")` | insertion order | no; the `StreamHandler` appends `"\n"` per record | log lines (section 11) |

### 0.3 Atomic write idioms

Three idioms are used. Rust must reproduce the same visible semantics:

1. **Plain tmp + rename, no fsync.** `tempfile.mkstemp(prefix=..., suffix=".tmp", dir=<same dir>)`, write, `os.replace(tmp, final)`, then in `finally` `os.unlink(tmp)` ignoring `FileNotFoundError`. Used by: manifest (`_atomic_write_json`), storms.json, ProcessedSet (`_flush`, which additionally cleans up and re-raises on exception), nowcast-skill.
2. **Durable tmp + rename, with fsync.** Same as (1) but `f.flush(); os.fsync(f.fileno())` before `os.replace`, and a directory fsync after. Used by: grid `*.meta.json` and the generation marker (`grid_dump.py`).
3. **Immutable data + pointer.** Grid binaries are written **directly** to a unique final name (`{ts}.{gen32}.bin`) with `open("wb")` and fsync; nothing reads them until the pointer (`*.meta.json`) is atomically replaced. See section 5.

`mkstemp` creates files with mode `0600`. Every file written through `mkstemp` (manifest, storms.json, meta, marker, ProcessedSet, nowcast-skill) therefore ends up `0600` after rename. The grid `.bin` is created with `open("wb")` and gets the default mode (`0666 & ~umask`, typically `0644`). **[HAZARD]** If the API or another process runs as a different user, this matters. A Rust port should choose modes explicitly; the spec's default is "match Python" (`0600` for `mkstemp`-based files, umask default for `.bin`).

### 0.4 Locks

All locks are **BSD `flock(2)`** via `fcntl.flock(fd, LOCK_EX)`, **blocking**, on sidecar files that are created with `open("w")` / `open("a+b")` and never deleted:

| Lock file | Path | Guards |
|---|---|---|
| `manifest.lock` | `<STATE_DIR>/manifest.lock` (from `manifest_path().with_suffix(".lock")`) | read-modify-write of `manifest.json` |
| `storms.lock` | `<state_dir>/storms.lock` | read-compare-write of `storms.json` |
| `.grid.lock` | `<GRID_DIR>/<layer>/.grid.lock` | every grid metadata commit, generation marker write, and prune for that layer |

Rust MUST use `flock(2)` semantics (for example `rustix::fs::flock` or `fs4`), **not** `fcntl` record locks, so it interoperates with any remaining Python writer. The lock is released explicitly on the success path (`LOCK_UN`) and implicitly when the fd closes on error paths. Lock files are not removed.

### 0.5 Environment variables (module-level constants)

Each value is read once, at import. A Rust port should read them at startup and keep the same names and defaults.

| Variable | Default | Module | Meaning |
|---|---|---|---|
| `STATE_DIR` | `/data/state` | manifest, push_tokens, nowcast/activities | state directory |
| `GRID_DIR` | `/data/grids` | grid_dump, storm_watch | grid root |
| `GRID_MAX_AGE_S` | `43200` (12 h) | grid_dump | age sweep cutoff |
| `GRID_MAX_CELLS` | `810000` (900*900) | grid_dump | default cell cap for `write_grid` |
| `GRID_ORPHAN_GRACE_S` | `300`, clamped to `>= 0` | grid_dump | grace before removing unreferenced `.bin` |
| `NOWCAST_INPUT_FRAMES` | `4`, clamped to `>= 3` | grid_dump | keep-N input |
| `NOWCAST_INPUT_STEP_MIN` | `5` (float) | grid_dump | keep-N input |
| `NOWCAST_GRID_INPUT_LAYER` | `radar-nowcast-input` | grid_dump | layer name for keep-N |
| `STORM_THRESHOLD_DBZ` | `40` | storms | cell threshold |
| `STORM_MIN_PIXELS` | `5` | storms | min component size |
| `STORM_MAX_CELLS` | `500` | storms | max features kept |
| `STORM_MAX_TRACK_SPEED_KMH` | `160` | storms | speed clamp and gate term |
| `STORM_MATCH_SLACK_KM` | `8` | storms | association gate floor |
| `STORM_TRACK_HISTORY_MIN` | `20` | storms | history window |
| `STORM_MIN_TRACK_SPAN_MIN` | `5` | storms | min span for motion |
| `STORM_PREFETCH_PADDING_KM` | `12` | storms | bbox padding |
| `STORM_PREFETCH_MAX_DISTANCE_KM` | `500` | storm_prefetch | max user-to-cell distance |
| `WATCH_RADIUS_KM` | `20` | storm_watch | (declared; callers pass radius explicitly) |
| `INTENSIFY_DBZ_DELTA` | `10` | storm_watch | |
| `DISSIPATE_DBZ_DELTA` | `10` | storm_watch | |
| `HAIL_DBZ_THRESHOLD` | `60` | storm_watch | |
| `NOWCAST_SKILL_WINDOW_HOURS` | `24` | nowcast_skill | rolling window |
| `NOWCAST_SKILL_HEADLINE_LEAD_MIN` | `30` | nowcast_skill | headline lead target |
| `NOWCAST_SKILL_MIN_RUNS` | `6` | nowcast_skill | min runs before headline |
| `NOWCAST_SKILL_MATCH_TOLERANCE_S` | `150` | nowcast/skill.py | valid-time match tolerance |
| `LOG_LEVEL` | `INFO` | logger | root level per service |

---

## 1. Module map and dependency graph

```
manifest.py  <- api (serve manifest.json), tile_cleanup, ingest_mrms, ingest_hrrr, ingest_airquality,
                nowcast/activities, storm_prefetch (read only)
state.py     <- ingest_mrms, ingest_hrrr, ingest_airquality, nowcast/activities (ProcessedSet)
grid_dump.py <- ingest_mrms, ingest_hrrr, ingest_airquality, nowcast/activities (write/finalize/prune),
                storm_watch (read via meta files), nowcast/skill.py (reads meta/bin), api inspector (read)
storms.py    <- ingest_mrms (write_storms_json, radar only); test_storms.py
storm_prefetch.py <- api /api/storm-prefetch route (per ARCHITECTURE.md); imports manifest.read_manifest_file
storm_watch.py    <- api/storm_watch_activities.py (latest_radar_meta, sample_window, detect_change)
push_tokens.py    <- API token store (caller not traced in this pass)
nowcast_skill.py  <- nowcast/skill.py (append_entries); API summary and /api/metrics (read_log, summarize, prometheus_lines, per module docstring)
logger.py    <- every service (get_logger, retry)
activity_heartbeat.py <- ingest_mrms/hrrr/airquality, nowcast (run_sync_with_heartbeat)
```

---

## 2. Cross-cutting: directory layout on the PVC

```
<STATE_DIR>/                        (default /data/state)
  manifest.json                     schema v2 (section 3)
  manifest.lock                     flock sidecar
  storms.json                       last written storm payload (section 6)
  storms.lock
  nowcast-skill.json                rolling verification log (section 10)
  nowcast.json                      ProcessedSet of nowcast anchors (max 100)
  ingest-mrms-<layer>.json          ProcessedSet per MRMS layer (max 2000)
  ingest-hrrr.json                  ProcessedSet (max 200)
  ingest-airquality.json            ProcessedSet (max 100)
  push_tokens.sqlite                (+ default SQLite journal files) (section 9)

<TILE_DIR>/                         (tiles; out of scope, referenced only for existence checks)
  <layer>/<palette>/<path...>/<z>/<x>/<y>.png

<GRID_DIR>/                         (default /data/grids)
  <layer>/
    .grid.lock
    <grid_key>.meta.json            commit pointer (section 5.4)
    <grid_key>.<gen32>.bin          immutable Float32 data (section 5.3)
    <grid_key>.bin                  legacy unversioned data (readable, prunable)
    .<name>.<rand>.tmp              in-flight temporary (skipped by prune)
    runs/<run>/                     run-scoped writers (nowcast, hrrr, aq)
      <valid>.meta.json, <valid>.<gen32>.bin, observed.meta.json, ...
      .grid-generation-complete-v1.json   completion marker (section 5.6)
```

`grid_key` may contain `/`, which creates subdirectories. Validation is in `_relative_grid_key` (section 5.8).

---

## 3. Manifest (`manifest.py`)

### 3.1 Constants

```
MANIFEST_SCHEMA_VERSION = 2
TILE_URL_TEMPLATE        = "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png"
TILE_URL_TEMPLATE_LEGACY = "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png"
STATE_DIR                = os.environ.get("STATE_DIR", "/data/state")   # str
```

### 3.2 File location

- `manifest_path(state_dir=None) -> Path` = `Path(state_dir or STATE_DIR) / "manifest.json"`.
- Lock file = `manifest.lock` in the same directory (`path.with_suffix(".lock")`).
- The API serves `manifest.json` directly (the design goal is that request paths never crawl the tile PVC). It is read by clients at `/api/manifest.json` (15 s server cache, 30 s client poll per ARCHITECTURE.md).

### 3.3 Schema v2, full key list

Top level (written with `sort_keys`, so file order is alphabetical):

| Key | Type | Required | Writer behavior |
|---|---|---|---|
| `layers` | object `{<layerName>: Layer}` | yes | map keyed by layer name |
| `schema_version` | integer | yes | always `2` after any write. Read default `1` when absent (3.6) |
| `tile_url_template` | string | yes | `TILE_URL_TEMPLATE` (default set on read if missing) |
| `tile_url_template_legacy` | string | yes | `TILE_URL_TEMPLATE_LEGACY` (default set on read if missing) |
| `updated_at` | string (ISO-8601 with offset) | yes | `datetime.now(timezone.utc).isoformat()` at every write |

**Layer object** (keys written by `_sync_layer_indexes` plus caller-supplied metadata):

| Key | Type | Source |
|---|---|---|
| `frames` | array of Frame, sorted by `timestamp` ascending, unique | maintained by add/remove/replace |
| `timestamps` | array of string | derived: `[frame.timestamp for frame in frames]` |
| `latest` | string | derived: last frame's timestamp. **Absent** when no frames |
| `palettes` | array of string, sorted | derived: sorted intersection of every frame's `palettes`. `[]` when no frames |
| `title` | string | caller `layer_metadata` (e.g. `"MRMS observed reflectivity"`) |
| `kind` | string | caller `layer_metadata` (`observation`, `nowcast`, `model_guidance`) |
| `horizon_minutes`, `step_minutes`, `method` | int, int, string | nowcast `layer_metadata` |
| `run_id` | string | nowcast / hrrr / aq `layer_metadata` |
| `complete` | boolean | hrrr / aq `layer_metadata` |

Any other key supplied in `layer_metadata` is stored verbatim. Metadata keys are merged with `dict.update`, **before** `_sync_layer_indexes`, so a metadata key named `frames`, `timestamps`, `latest` or `palettes` is overwritten by the sync. Callers do not do this.

**Frame object**:

| Key | Type | Required | Notes |
|---|---|---|---|
| `timestamp` | string | yes | identity; unique within a layer |
| `path` | string | yes | tile path segment. Defaults to `timestamp` when missing or empty (`str(frame.get("path") or timestamp)`) |
| `palettes` | array of string, sorted | yes | `sorted({str(p) for p in ...})`. See the precedence rule in 3.4 |
| any other key | any JSON | no | passed through from `frame=` (see below) |

Frame keys emitted by real callers (for the port's typed model):

| Caller | Layer | `path` | Extra frame keys |
|---|---|---|---|
| `ingest_mrms` (radar, radar-composite) | observed MRMS | `<timestamp>` | `source: "mrms"`, `kind: "observation"`, `issued_at` (= timestamp), `lead_minutes: 0`, `spatial_resolution_km: 1.0`, `max_zoom: 7` |
| `nowcast/activities.py` `_render_lead` | `nowcast` | `runs/<anchor>/<valid>` | `timestamp`, `path`, `grid_key` (= `runs/<anchor>/<valid>`), `source: "mrms-nowcast"`, `kind: "nowcast"`, `issued_at` (anchor isoformat), `lead_minutes` (int, `(i+1)*STEP_MIN`), `input_interval_minutes` (float, rounded to 3 dp), `method` (`"pysteps-sprog"`), `spatial_resolution_km` (float rounded 2 dp), `max_zoom: 6` |
| `ingest_hrrr` | `radar-hrrr` and other HRRR layers | `runs/<run_id>/<valid>` | `source: "hrrr"`, `kind: "model_guidance"`, `issued_at`, `run_id`, `lead_minutes` (= `fhr*60`), `spatial_resolution_km: 3.0`, `max_zoom` |
| `ingest_airquality` | `air-quality`, `ozone` | `runs/<run_id>/<ts>` | `source: "aqm"`, `kind: "model_guidance"`, `issued_at`, `run_id`, `lead_minutes` (int, `floor(minutes)`), `spatial_resolution_km: 5.0`, `max_zoom` |

Frame `source` values are free strings. The client `RadarFrame.source` type lists only `radar | nowcast | radar-hrrr`, but the web client takes the layer key as `source`. The manifest `source` field is informational.

### 3.4 Functions

All public functions take `state_dir` as `str | Path | None` and resolve it via `manifest_path`.

**`empty_manifest() -> dict`**
Returns `{schema_version: 2, layers: {}, tile_url_template, tile_url_template_legacy, updated_at: now-ISO}`. Note that `updated_at` is non-deterministic.

**`read_manifest_file(state_dir=None) -> dict`**
- Reads the file with `Path.read_text()` (UTF-8 by default in CPython with a UTF-8 locale; treat as UTF-8).
- `FileNotFoundError` -> `empty_manifest()`.
- Any other `OSError` (EIO, EISDIR, EACCES) -> **raise**.
- JSON parse failure (`json.JSONDecodeError`, including a zero-byte file) -> **raise**. The test asserts this, and the writer also raises, so the file is never rewritten with a single layer.
- Parsed body that is not a `dict`, or whose `layers` is not a `dict` -> `empty_manifest()`. **[DEFECT]** Unlike a parse failure, this silently returns an empty document. The next `update_manifest_file` then overwrites the file and drops every layer. See 14.
- Defaults (only when keys are missing, the file is not rewritten by reading): `tile_url_template`, `tile_url_template_legacy`, `updated_at` (now), `schema_version` = **1**.
- **v1 migration on read**: for each layer dict whose `frames` is not a list, synthesize frames from `timestamps`: `{"timestamp": ts, "path": ts, "palettes": list(layer.get("palettes", ["classic"]))}` for each `ts` in `str(timestamps)`. Layers that are not dicts are skipped. Note that a layer whose `palettes` key is `[]` yields frames with `palettes: []`.
- Returned body is not normalized further (no sorting or index sync on read).

**`_normalize_frame(timestamp, *, palettes, frame) -> dict`** (private)
```
normalized = dict(frame or {})
normalized["timestamp"] = timestamp
normalized["path"] = str(normalized.get("path") or timestamp)
normalized["palettes"] = sorted({str(p) for p in (palettes or normalized.get("palettes") or ["classic"])})
```
**Palette precedence**: explicit non-empty `palettes` argument > the frame's own `palettes` > the literal default `["classic"]`. An explicit empty list or `None` falls through. The default `["classic"]` is silent: adding a frame with no palettes records `classic`. Extra frame keys are preserved.

**`_sync_layer_indexes(layer) -> None`** (private)
1. `frames_by_timestamp = {str(f["timestamp"]): f for f in layer["frames"] if dict and f.get("timestamp")}`. For duplicate timestamps the **last** occurrence wins.
2. `frames = [frames_by_timestamp[k] for k in sorted(frames_by_timestamp)]` (string sort).
3. `layer["frames"] = frames`; `layer["timestamps"] = [str(f["timestamp"]) for f in frames]`.
4. If frames non-empty: `layer["latest"] = frames[-1]["timestamp"]`; `layer["palettes"] = sorted(intersection of set(f.get("palettes", [])) over frames)`.
   If frames empty: `layer.pop("latest")`; `layer["palettes"] = []`.

**`update_manifest_file(layer_name, timestamp, *, palettes=None, action="add", state_dir=None, frame=None, layer_metadata=None) -> dict`**
Algorithm, under `manifest.lock` (LOCK_EX):
1. `path.parent.mkdir(parents=True, exist_ok=True)`; open `manifest.lock` with `"w"`; `flock(LOCK_EX)`.
2. `manifest = read_manifest_file(path.parent)` (may raise; the lock is released when the fd closes).
3. `layers = manifest.setdefault("layers", {})`.
4. If `action == "add"`:
   - `layer = layers.setdefault(layer_name, {"timestamps": [], "frames": [], "palettes": []})`.
   - `existing = [c for c in layer.get("frames", []) if dict and c.get("timestamp") != timestamp]` (removes any existing frame with the same timestamp, which makes add idempotent-replace).
   - Append `_normalize_frame(timestamp, palettes=palettes, frame=frame)`.
   - `layer["frames"] = existing`. If `layer_metadata` truthy: `layer.update(layer_metadata)`. Then `_sync_layer_indexes(layer)`.
5. If `action == "remove"`:
   - If the layer exists: filter out frames with that timestamp; `_sync_layer_indexes`; if no frames remain, `layers.pop(layer_name)`. Missing layer is a no-op. `palettes`, `frame`, `layer_metadata` are ignored.
6. Any other action -> `ValueError(f"unsupported manifest action: {action}")` (raised before writing; lock released on close).
7. `manifest["schema_version"] = 2`; `manifest["updated_at"] = now-ISO`.
8. `_atomic_write_json(path, manifest)` (3.5); unlock; return the manifest.

Note: a **remove** still rewrites the file and bumps `updated_at` even when nothing changed.

**`replace_layer_manifest(layer_name, timestamps, *, palettes=None, state_dir=None, frames=None, layer_metadata=None) -> dict`**
Used for forecast layers (nowcast, hrrr, aq), where each run supersedes the previous. Algorithm under the same lock:
1. Read manifest as above.
2. If `timestamps` is non-empty:
   - `layer = layers.setdefault(layer_name, {"timestamps": [], "frames": [], "palettes": []})`. **Unknown keys already on the layer are kept.**
   - `supplied = {str(f["timestamp"]): f for f in frames if dict and f.get("timestamp")}`.
   - `layer["frames"] = [_normalize_frame(ts, palettes=palettes, frame=supplied.get(ts)) for ts in sorted(set(timestamps))]`. Note that the **explicit `palettes` overrides every frame's own `palettes`** (when non-empty). Supplied `frames` entries whose timestamp is not in `timestamps` are ignored.
   - If `layer_metadata` truthy: `layer.update(...)`. Then `_sync_layer_indexes(layer)`.
3. Else (`timestamps` empty): `layers.pop(layer_name, None)`. `frames` and `layer_metadata` are ignored.
4. `schema_version = 2`, `updated_at = now`, atomic write, unlock, return.

Replace always replaces the full frame list (old frames are dropped even if their tiles still exist on disk; cleanup handles tiles).

**`_atomic_write_json(path, body)`** (private)
`mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)`; `json.dump(body, fh, separators=(",", ":"), sort_keys=True)`; `"\n"`; `os.replace(tmp, path)`; in `finally` unlink tmp, ignoring `FileNotFoundError`. **No fsync** (see 14). Writer style A (0.2).

### 3.5 Layer and frame lifecycle (summary)

| Action | Trigger | Effect on `layers[name]` |
|---|---|---|
| add frame | MRMS radar/composite per frame (`update_manifest_file(..., action="add")`) | append or replace same timestamp; reindex; layer created on first frame |
| remove frame | tile cleanup (`tile_cleanup/activities.py:117`), MRMS and HRRR stale-dir sweeps (`update_manifest_file(layer, ts_dir.name, action="remove")`) | drop frame; reindex; drop layer when empty |
| replace layer | nowcast each run (after tiles + grids are complete), hrrr per run, aq per run | replace all frames; metadata merged; drop layer if list empty |

Pruning of frames is by remove only. Replace does not prune by age. Frames older than retention are removed by tile cleanup, which is outside shared.

### 3.6 Schema version handling

- Writers always stamp `schema_version: 2`.
- Readers treat a missing `schema_version` as 1 and migrate frames (3.4). Readers do not reject any version, so a future `3` would be read as-is.
- The client validator (`parseSelfHostedManifest`) accepts any integer `schema_version >= 1` when present (section 3.10).

### 3.7 Example document

Pretty-printed for the spec. The real file is one line plus `\n`, with sorted keys.

```json
{
  "layers": {
    "radar": {
      "frames": [
        {
          "issued_at": "2026-05-20T12:00:00+00:00",
          "kind": "observation",
          "lead_minutes": 0,
          "max_zoom": 7,
          "palettes": ["classic", "vivid"],
          "path": "2026-05-20T12:00:00+00:00",
          "source": "mrms",
          "spatial_resolution_km": 1.0,
          "timestamp": "2026-05-20T12:00:00+00:00"
        }
      ],
      "kind": "observation",
      "latest": "2026-05-20T12:00:00+00:00",
      "palettes": ["classic", "vivid"],
      "timestamps": ["2026-05-20T12:00:00+00:00"],
      "title": "MRMS observed reflectivity"
    },
    "nowcast": {
      "frames": [
        {
          "grid_key": "runs/2026-05-20T12:00:00+00:00/2026-05-20T12:05:00+00:00",
          "input_interval_minutes": 2.0,
          "issued_at": "2026-05-20T12:00:00+00:00",
          "kind": "nowcast",
          "lead_minutes": 5,
          "max_zoom": 6,
          "method": "pysteps-sprog",
          "palettes": ["classic"],
          "path": "runs/2026-05-20T12:00:00+00:00/2026-05-20T12:05:00+00:00",
          "source": "mrms-nowcast",
          "spatial_resolution_km": 1.0,
          "timestamp": "2026-05-20T12:05:00+00:00"
        }
      ],
      "horizon_minutes": 60,
      "kind": "nowcast",
      "latest": "2026-05-20T12:05:00+00:00",
      "method": "pysteps-sprog",
      "palettes": ["classic"],
      "run_id": "2026-05-20T12:00:00+00:00",
      "step_minutes": 5,
      "timestamps": ["2026-05-20T12:05:00+00:00"],
      "title": "MRMS motion nowcast"
    }
  },
  "schema_version": 2,
  "tile_url_template": "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
  "tile_url_template_legacy": "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
  "updated_at": "2026-05-20T12:05:03.418221+00:00"
}
```

Note: the nowcast example frame is illustrative. Real nowcast frames carry the same keys as `_render_lead`, so the example matches.

### 3.8 Behavior fixtures (from `test_manifest.py`)

- Two adds, then remove one: `timestamps` equals the remaining list, `latest` equals the last remaining.
- After adds with `palettes=["classic"]` then `["vivid"]`: layer `palettes == []` (intersection), frame 0 `palettes == ["classic"]`, frame 1 `palettes == ["vivid"]`.
- `replace_layer_manifest("nowcast", [t1, t2], palettes=["classic","vivid"])`: `palettes == ["classic","vivid"]`, `latest == t2`, other layers untouched.
- `replace_layer_manifest("nowcast", [])` removes the layer.
- Missing file: `read_manifest_file` returns `layers == {}`.
- Corrupt JSON (`'{"layers": {"radar": '`): read raises `JSONDecodeError`; `update_manifest_file` also raises and leaves the file byte-identical.
- Path is a directory: read raises `OSError`.

### 3.9 Hazards

- **Lexicographic sort** (3.4). Mixed offsets (`Z` vs `+00:00`, or `-05:00`) produce an order that differs from chronological. The client requires chronological order (`Date.parse` non-decreasing). Writers MUST use UTC `+00:00`. The Rust writer should normalize or reject non-UTC input.
- **Duplicate timestamps across an add**: the add filter removes the old frame, so the new frame's metadata wins.
- **Frames replace per-timestamp palette**: `_normalize_frame` takes the explicit `palettes` argument over any frame value, so `replace_layer_manifest(..., palettes=X, frames=[{..., "palettes": Y}])` uses `X`.
- **Read-then-write without partial trust**: any failed read (other than missing file) aborts the write.

### 3.10 Client validation rules (`frontend/src/lib/manifest.ts`)

The Rust server SHOULD produce documents that pass these. They are the contract the app enforces before caching.

- Root: `layers` object; `tile_url_template` non-empty string (<= 512 chars); `updated_at` ISO timestamp; `schema_version` optional integer >= 1.
- Layer name: non-empty string <= 64 chars.
- Layer: `timestamps` array (required). Each item is an ISO timestamp, must be **unique** and **non-decreasing by `Date.parse`**.
- Layer `frames` (optional): array; length and order must equal `timestamps` exactly (`frames[i].timestamp === timestamps[i]`).
- Layer optional: `latest` (ISO), `title` (<= 256 chars), `kind` (<= 64 chars), `complete` (boolean).
- Frame: `timestamp` ISO (required); `path` required and "safe": not starting with `/`, no `\`, `?`, `#`, and every `/`-segment non-empty and not `.` or `..`. `source` optional <= 64 chars. `kind` optional, one of `observation | nowcast | model_guidance`. `issued_at` optional ISO. `lead_minutes` optional finite number >= 0. `spatial_resolution_km` optional finite number > 0. `max_zoom` optional integer in `[0, 24]`. `palettes` optional array of strings each <= 128 chars, non-empty.
- ISO timestamp regex: `YYYY-MM-DDTHH:MM:SS(.1-9 digits)?(Z|±HH:MM)`, with year in [2000, 2200], a valid day-of-month, and `Date.parse` finite.

Consequence for the port: `path` values like `runs/<anchor>/<valid>` are valid. The `+` and `:` characters in timestamps are allowed in `path` by this validator.

---

## 4. ProcessedSet (`state.py`)

### 4.1 Purpose

A bounded, disk-backed set of string keys that have been processed (MRMS keys, HRRR run ids, nowcast anchors). It is keyed by caller-supplied `path`.

### 4.2 Signature

```
class ProcessedSet:
    def __init__(self, path: Path | str, max_entries: int = 5000)
    def __contains__(self, key: object) -> bool
    def add(self, key: str) -> None
    def update(self, keys: Iterable[str]) -> None
    # internal: _load(), _flush()
```

Public attributes: `path`, `max_entries`. Internal: `_items: list[str]` (insertion order), `_set: set[str]`.

### 4.3 File format (writer style C)

```
{"items": ["<key1>", "<key2>", ...]}
```

- Written with `json.dump({"items": self._items}, f)`: default separators `", "` and `": "`, **no trailing newline**, insertion order (oldest first).
- Example: `{"items": ["2026-05-20T12:00:00+00:00", "2026-05-20T12:02:00+00:00"]}`.

### 4.4 Load semantics (`_load`)

- If the file does not exist: empty set.
- `json.loads(read_text())`. `items = data.get("items", []) if isinstance(data, dict) else list(data)`.
  - If the file is a JSON **array**, it is the item list (legacy form).
  - If it is a dict without `items`, items = `[]`.
  - `_items = [str(x) for x in items]`; `_set = set(_items)`.
- On `json.JSONDecodeError` or `OSError`: reset to empty (silently). A corrupt file is replaced by the next write.
- **[DEFECT]** Any other exception from the `list(data)` / `[str(x) ...]` step propagates out of the constructor. Examples: a top-level JSON number, `null`, or `{"items": 5}`. A top-level JSON string is accepted character by character (`list("abc")`). See 14.
- Loading does **not** trim to `max_entries`. A file with more entries than the cap stays oversized until the next insertion that adds a new key.
- Loading does not dedupe `_items`. Duplicates in the file are kept in `_items` but collapse in `_set`.

### 4.5 Membership and insertion

- `__contains__` tests `_set`.
- `add(key)`: if `key in _set`, return (no write). Else append to `_items`, add to `_set`. If `len(_items) > max_entries`: `dropped = _items[:-max_entries]`; `_items = _items[-max_entries:]`; discard each dropped key from `_set`. Then `_flush()`. Every new key causes a full rewrite.
- `update(keys)`: for each key not already in `_set`: append and add (duplicates within the input are collapsed). If anything was added: apply the same trim, then one `_flush()`. No write when nothing new.
- **[HAZARD]** `max_entries = 0`: the slice `_items[:-0]` is `_items[:0]` (empty), and `_items[-0:]` is the whole list. So `max_entries=0` means **unbounded**, not "store nothing". Nothing in the repo uses 0. Negative values are not guarded.
- **[DEFECT]** Trimming discards from `_set` by key. If `_items` contains a duplicate of a dropped key that remains in the kept tail, `_set` loses the key while `_items` still holds it. Happens only when the loaded file had duplicates.

### 4.6 Flush (`_flush`)

1. `path.parent.mkdir(parents=True, exist_ok=True)`.
2. `mkstemp(dir=path.parent, prefix=".state-", suffix=".json.tmp")`.
3. Write `{"items": ...}` (style C). **No fsync.**
4. `os.replace(tmp, path)`.
5. On any exception: unlink tmp (ignoring `OSError`) and re-raise.

### 4.7 Concurrency

- **No lock.** Two processes with the same file each keep their own in-memory copy. The last `os.replace` wins and the other's entries are lost. The design assumes one writer per file. Callers name files per layer (`ingest-mrms-<layer>.json`) to avoid races between schedules (`ingest_mrms/activities.py:237`).
- Within one process, `ProcessedSet` objects are not shared. `nowcast/activities.py` constructs a new instance for each membership test, which reloads from disk.

### 4.8 Caller parameters

| Caller | Path | max_entries |
|---|---|---|
| `ingest_mrms` per layer | `STATE_DIR/ingest-mrms-<layer>.json` | 2000 |
| `ingest_hrrr` | `STATE_DIR/ingest-hrrr.json` | 200 |
| `ingest_airquality` | `STATE_DIR/ingest-airquality.json` | 100 |
| `nowcast` | `STATE_DIR/nowcast.json` | 100 |

---

## 5. Grid dumps (`grid_dump.py`) — byte-exact

Consumers: nowcast (reads `radar-nowcast-input`, `nowcast/skill.py` reads point grids and observed anchors), the API inspector (`/api/inspect/...`, bilinear sampling), and storm_watch (`sample_window`).

### 5.1 Constants (defaults)

```
GRID_DIR             = os.environ.get("GRID_DIR", "/data/grids")
GRID_MAX_AGE_S       = int(env GRID_MAX_AGE_S, 43200)
MAX_CELLS            = int(env GRID_MAX_CELLS, 900*900 = 810000)
GRID_ORPHAN_GRACE_S  = max(0, int(env GRID_ORPHAN_GRACE_S, 300))
_NOWCAST_FRAMES      = max(3, int(env NOWCAST_INPUT_FRAMES, 4))
_NOWCAST_STEP_MIN    = float(env NOWCAST_INPUT_STEP_MIN, 5)
GRID_KEEP_LAST = {
  env NOWCAST_GRID_INPUT_LAYER (default "radar-nowcast-input"):
      (_NOWCAST_FRAMES - 1) * max(1, ceil(_NOWCAST_STEP_MIN / 1.8)) + 3
}
```

With defaults: `(4-1) * max(1, ceil(5/1.8)) + 3 = 3*3 + 3 = 12`. So `GRID_KEEP_LAST["radar-nowcast-input"] == 12`. `_LAYER_LOCK_FILE = ".grid.lock"`, `_GENERATION_ROOT = "runs"`, `_GENERATION_COMPLETE_FILE = ".grid-generation-complete-v1.json"`.

`_DIRECTORY_FSYNC_UNSUPPORTED = {EINVAL, ENOTSUP}`.

### 5.2 Binary data file: byte-level format

- **No header, no footer, no compression.**
- Contents: `arr.astype(float32).tobytes(order="C")`. Total size: `height * width * 4` bytes exactly.
- Element type: IEEE 754 binary32, **native byte order** in Python. On every platform the project targets (x86_64, aarch64 Linux) that is **little-endian**. The reader uses `np.fromfile(..., dtype="<f4")`, which is explicit little-endian. The Rust port MUST read and write little-endian explicitly (`f32::from_le_bytes`).
- Layout: row-major (C order). Shape `(height, width)`. **Row 0 is the northernmost latitude** (`lat_max`). Column 0 is the westernmost longitude (`lon_min`). Latitude decreases with row index. Longitude increases with column index.
- Missing values: see 5.5 step 7 (NaN handling).
- Example, 2x2 float32 with all ones, file size 16 bytes:
  `00 00 80 3f | 00 00 80 3f | 00 00 80 3f | 00 00 80 3f`.

### 5.3 Meta sidecar: `<grid_key>.meta.json`

Writer style A (sorted keys, compact, trailing newline). Keys:

| Key | Type | Meaning |
|---|---|---|
| `data_file` | string | file name (no directory) of the `.bin` this meta points to |
| `fill` | float | no-data sentinel as stored in the binary. `-9999.0` when the caller's `fill` is NaN, else `float(fill)` |
| `height` | int | rows in the stored grid (after stride) |
| `width` | int | columns in the stored grid (after stride) |
| `lat_min`, `lat_max` | float | min and max of the stored `lats` (after stride), degrees |
| `lon_min`, `lon_max` | float | min and max of the stored `lons` (after stride), degrees (-180..180 after normalization) |
| `unit` | string | caller label, e.g. `"dBZ"`, `"°F"`, `"mph"`, `"J/kg"` (written as `°F`) |
| `stride` | int | decimation factor applied (power of 2, 1 when none) |

Keys sorted: `data_file, fill, height, lat_max, lat_min, lon_max, lon_min, stride, unit, width`.

Example for `write_grid("radar", "2026-07-15T12:00:00+00:00", np.ones((2,2)), lats=[43.0,42.0], lons=[-86.0,-85.0], "dBZ")`, with `gen = 9f0c...` (32 lowercase hex):

```
{"data_file":"2026-07-15T12:00:00+00:00.9f0c2a7e5b1d4c8e9a3f0b6d7e2c1a4f.bin","fill":-9999.0,"height":2,"lat_max":43.0,"lat_min":42.0,"lon_max":-85.0,"lon_min":-86.0,"stride":1,"unit":"dBZ","width":2}
```

`data_file` must not contain a path separator (`Path(value).name == value`), or readers ignore it (5.8).

### 5.4 Generation marker: `runs/<run>/.grid-generation-complete-v1.json`

Writer style A. Keys sorted: `generation, grid_count, grid_metadata, schema_version`.

```
{"generation":"<run>","grid_count":N,"grid_metadata":["<valid1>.meta.json", ...],"schema_version":1}
```

- `generation` = the run directory name.
- `grid_metadata` = **sorted** list of `*.meta.json` names in that directory (non-recursive).
- `grid_count` = its length.

### 5.5 `write_grid(...)` algorithm

Signature (Python):
```
write_grid(layer: str, timestamp: str, data: np.ndarray, lats: np.ndarray, lons: np.ndarray,
           unit: str, fill: float = nan, max_cells: int | None = None,
           grid_key: str | None = None) -> str | None
```
Returns the absolute-or-relative string of the `.bin` path, or `None` when the input shape is rejected.

Steps:
1. **Shape check.** Reject (return `None`, no write) unless `data.ndim == 2`, `lats.ndim == 1`, `lons.ndim == 1`, `h == len(lats)`, `w == len(lons)`.
2. **Latitude orientation.** If `lats[0] < lats[-1]`: `data = flipud(data)`, `lats = lats[::-1]`. Only the two endpoints are compared. Equal endpoints (for example length 1) do not flip.
3. **Longitude normalization.** If `max(lons) > 180.0`: `lons = where(lons > 180, lons - 360, lons)`; `order = argsort(lons)`; `lons = lons[order]`; `data = data[:, order]`. If no value exceeds 180, `lons` and `data` are left unchanged even if `lons` is not ascending. **[HAZARD]**
4. **Stride decimation.** `cell_limit = max(1, int(max_cells or MAX_CELLS))` (note `max_cells=0` falls back to `MAX_CELLS`). `stride = 1`; while `(h // stride) * (w // stride) > cell_limit`: `stride *= 2`. If `stride > 1`: `data = data[::stride, ::stride]`, `lats = lats[::stride]`, `lons = lons[::stride]`, `h, w = data.shape`.
5. **Destination.** `layer_dir = GRID_DIR / layer`; `out_base = layer_dir / _relative_grid_key(grid_key or timestamp)`. `meta_path = out_base.parent / f"{out_base.name}.meta.json"`.
6. **Dtype and fill.** `arr = contiguous float32 copy of data`.
   - If `fill` is NaN: `arr = where(isnan(arr), -9999.0, arr)`, `fill_val = -9999.0`.
   - Otherwise `fill_val = float(fill)` and **`arr` is not changed**. NaN cells remain NaN in the binary. **[HAZARD]** Only the NaN-fill path replaces NaN.
7. **Meta dict** (computed from the post-stride `lats`, `lons`): `height, width, lat_min = min(lats), lat_max = max(lats), lon_min = min(lons), lon_max = max(lons), unit, fill = fill_val, stride`. `generation = uuid4().hex` (32 lowercase hex). `bin_path = out_base.parent / f"{out_base.name}.{generation}.bin"`. `meta["data_file"] = bin_path.name`.
8. **Commit under lock** `_layer_lock(layer_dir)` (5.8):
   a. `_durable_mkdir(out_base.parent)` (creates and fsyncs the directory chain).
   b. Open `bin_path` with `"wb"`, write `arr.tobytes(order="C")`, `flush`, `fsync`. Then `_fsync_dir(out_base.parent)`. **The binary is written directly under its final, unique name.** Nothing reads it yet.
   c. `mkstemp(prefix=f".{meta_path.name}.", suffix=".tmp", dir=meta_path.parent)`. Write meta (style A) plus `"\n"`, `flush`, `fsync`. `os.replace(tmp, meta_path)`. `_fsync_dir(out_base.parent)`. `finally`: unlink tmp, ignoring `FileNotFoundError`.
9. Return `str(bin_path)`.

Properties:
- Readers see either the previous complete generation or the new one.
- The **previous** `.bin` is not deleted by write. Retention (5.7) removes it later.
- If step 8b fails, a partial `.bin` remains with no referencing meta. Orphan grace (5.7) handles it.
- A failure in 8a or 8b propagates (tests check that an fsync `EIO` on the parent directory propagates).

### 5.6 `finalize_grid_generation(layer, generation) -> str`

Marks a run directory as complete. Under the layer lock:
1. `relative = _relative_grid_key(generation)`; must have exactly one path part (no `/`).
2. `generation_dir = GRID_DIR/<layer>/runs/<generation>`. If not a directory: `FileNotFoundError`.
3. `meta_names = sorted(*.meta.json names)`. If empty: `ValueError("grid generation has no metadata ...")`.
4. Write the marker (5.4) via tmp + fsync + replace + directory fsync.
5. Return `str(generation_dir)`.

Called by nowcast **before** the manifest swap, so a crash in between leaves the old manifest readable.

### 5.7 Retention

Three entry points. All run under the layer lock, except the age sweep in `cleanup_old_grids` which takes the lock per layer too.

**(a) `prune_grid_layer(layer, keep=None) -> int`**
- `keep = GRID_KEEP_LAST.get(layer)` if `keep is None`. If still `None`, return 0 (only layers with configured keep-N are pruned; `"radar"` is a no-op).
- If the layer directory does not exist: 0. Errors (`OSError`) return 0.
- Calls `_prune_grid_layer_unlocked(layer_dir, max(0, keep), now)`.

**(b) `_prune_grid_layer_unlocked(layer_dir, keep, now) -> int`** (caller holds the lock)
1. `entries = [p for p in layer_dir.iterdir() if p.is_file()]`. Symlinks followed. `OSError` -> 0.
2. `metas = sorted(names ending in ".meta.json")` (sort by full file name).
3. `kept_metas = metas[-keep:]` if `keep > 0`, else `[]`.
4. `keep_names = {kept meta names} ∪ {data_file of each kept meta}`.
5. `meta_names = {all meta names}`. `committed_data_names = {data_file of every meta (kept or not)}`.
6. For each file in `entries`:
   - skip `.grid.lock`;
   - skip if in `keep_names`;
   - skip names that start with `.` and end with `.tmp` (in-flight);
   - consider only if name ends with `.bin` or is a meta name, else skip (other files are untouched);
   - if name ends with `.bin` and is **not** in `committed_data_names` (orphan or legacy): skip if `now - mtime < GRID_ORPHAN_GRACE_S` (rolling-upgrade safety); on `stat` error skip;
   - `unlink`; increment on success; ignore `OSError`.
7. Return removed count. Each removed meta and each removed bin counts as one.

Consequence: a non-kept meta is removed, and so is its data file (it is in `committed_data_names`, so the grace rule does not apply).

**(c) `prune_grid_generations(layer, *, keep, active_generation=None) -> int`**
Used for run-scoped layers (nowcast point grids, `POINT_GRID_RETENTION_RUNS` = 36 by default).
1. If `active_generation` given: `_relative_grid_key`, must be one path part.
2. `generation_root = GRID_DIR/<layer>/runs`. If not a directory: 0.
3. Under lock: `complete = sorted(dirs under runs/ that are not symlinks and pass _is_complete_grid_generation, by name)`.
4. `retained = set(complete[-keep:])` if `keep > 0` else empty set. If `active_generation`: add `runs/<active_generation>` even if not complete or not in the newest `keep`.
5. For each complete directory not retained: count files (`rglob("*")`, files or symlinks), then `rmtree`. Add the count.
6. If anything was removed: `_fsync_dir(generation_root)`.
7. Return count. **In-progress directories (no marker) are never removed here.**

**(d) `cleanup_old_grids() -> int`**
1. If `GRID_DIR` does not exist: 0. `cutoff = now - GRID_MAX_AGE_S`.
2. For each child directory `layer_dir` of `GRID_DIR` (unsorted iteration):
   - Under `_layer_lock(layer_dir)`:
     a. If `GRID_KEEP_LAST` has the layer name: `removed += _prune_grid_layer_unlocked(layer_dir, max(0, keep), time.time())`.
     b. `entries = list(layer_dir.iterdir())` (`OSError` -> skip layer). For each entry except `.grid.lock`: if `stat().st_mtime < cutoff`: `unlink()`. A directory entry raises `IsADirectoryError` on `unlink`, which is swallowed. So **run directories (`runs/`) are never removed by the age sweep.**
   - Any `OSError` for the layer (for example `lost+found` that cannot host the lock) skips that layer.
3. Return total removed.

Note that the age sweep also removes current `*.meta.json` and `*.bin` for a quiet layer after 12 h without writes. That is the intended expiry. Clients do not see those timestamps because the manifest is what they consult, and tile cleanup removes manifest entries separately.

### 5.8 Helpers

- `_relative_grid_key(value) -> Path`: `parts = value.split("/")`. Raises `ValueError` if the value is empty, any part is empty, `.` or `..`, or the resulting path is absolute. Accepts backslashes and colons as ordinary characters.
- `_grid_timestamp_of(name) -> str`:
  - `"{ts}.meta.json"` -> `ts`.
  - `"{ts}.{gen}.bin"` where `gen` is exactly 32 lowercase hex characters -> `ts` (split on the last `.`).
  - Other `"{stem}.bin"` (legacy) -> `stem`.
  - Otherwise -> the name unchanged.
- `_read_meta_data_file(meta_path) -> str`: reads the meta JSON. On `OSError`, `ValueError` or `TypeError`, the document is `None`. If the document is a dict and `data_file` is a string whose `Path(value).name == value`, return it. Otherwise return `f"{_grid_timestamp_of(meta_path.name)}.bin"`.
- `_is_complete_grid_generation(path) -> bool`: reads `runs/<x>/.grid-generation-complete-v1.json`. True only if `schema_version` (coerced to int) == 1, `generation == path.name`, `grid_count` (int) == count of `*.meta.json` in the directory now, and `grid_metadata == sorted(meta names now)`. Any `OSError`, `KeyError`, `TypeError` or `ValueError` gives False. **Adding a meta to a finalized generation invalidates it.**
- `_layer_lock(layer_dir)`: `_durable_mkdir(layer_dir)`; open `layer_dir/.grid.lock` with `"a+b"`; `flock(LOCK_EX)`; yield; `finally` `flock(LOCK_UN)`.
- `_durable_mkdir(path)`: `path = absolute(path)`. Find the nearest existing ancestor. `mkdir(parents=True, exist_ok=True)`. Then `_fsync_dir` each directory from `path` up to and including that ancestor, stopping at the root.
- `_fsync_dir(path)`: `os.open(path, O_RDONLY | O_DIRECTORY)`. `os.fsync(fd)`. Ignore `OSError` whose errno is in `{EINVAL, ENOTSUP}` on either open or fsync. Re-raise others (test: `EIO` is not suppressed; `EINVAL` is).

### 5.9 Concurrency and crash semantics

- All metadata commits, marker writes and prune operations for a layer serialize on `.grid.lock`. The grid binary is written under the lock too (step 8).
- Readers take no lock. They rely on the atomic pointer.
- A prune waits for an in-progress commit (test `test_prune_waits_for_grid_metadata_commit`). The prune must not run until `os.replace` has returned, so it cannot remove the just-committed generation.
- Crash after step 8b and before 8c leaves an unreferenced `.bin`. Grace rule (5.7b) removes it later.
- Crash after 8c: committed. The old `.bin` is removed by retention.

### 5.10 Example retention outcomes (from tests)

- `prune_grid_layer("radar-nowcast-input", keep=4)` with 8 committed timestamps removes 8 files (4 metas + 4 bins).
- Orphan `.bin` with mtime older than the grace removed. Legacy `{ts}.bin` removed (count 2 with the orphan). In-flight `.<ts>.meta.json.<rand>.tmp` kept.
- `keep=0` removes all committed (6 for 3 stamps: 3 metas + 3 bins), keeps `.grid.lock`.
- `cleanup_old_grids` with `GRID_KEEP_LAST = {"radar-nowcast-input": 2}`, 5 stamps in both layers: removed 6 (nowcast 3 metas + 3 bins). `radar` untouched (no keep, young).

---

## 6. Storm cells and tracking (`storms.py`)

### 6.1 Constants

```
THRESHOLD_DBZ          = float(env STORM_THRESHOLD_DBZ, 40)
MIN_PIXELS             = int(env STORM_MIN_PIXELS, 5)
MAX_STORMS             = int(env STORM_MAX_CELLS, 500)
MAX_TRACK_SPEED_KMH    = float(env STORM_MAX_TRACK_SPEED_KMH, 160)
MATCH_SLACK_KM         = float(env STORM_MATCH_SLACK_KM, 8)
TRACK_HISTORY_MIN      = float(env STORM_TRACK_HISTORY_MIN, 20)
MIN_TRACK_SPAN_MIN     = float(env STORM_MIN_TRACK_SPAN_MIN, 5)
PREFETCH_PADDING_KM    = float(env STORM_PREFETCH_PADDING_KM, 12)
PREDICTION_MINUTES     = (0, 5, 10)
EARTH_RADIUS_KM        = 6371.0088
```
Also literal constants: latitude km per degree **110.574** (storms and storm_prefetch), longitude km per degree at equator **111.320**. The latter is written as `111.320 * cos(lat)`. (storm_watch uses 111.32 for latitude; see 14.)

### 6.2 Public API

```
detect_storms(data: np.ndarray, lats: np.ndarray, lons: np.ndarray, *,
              timestamp: str, previous: dict | None = None) -> dict
write_storms_json(state_dir: Path, data, lats, lons, timestamp: str) -> None
```
Private helpers: `_parse_timestamp`, `_haversine_km(a, b)` (a and b are `(lon, lat)` tuples), `_component_bbox`, `_translate_bbox`, `_previous_motion`, `_advance`, `_fit_motion`, `_attach_tracking`.

### 6.3 Detection algorithm (`detect_storms`)

Inputs: `data` shape `(H, W)` float (32-bit in practice); `lats` length `H`; `lons` length `W`. The function is orientation-agnostic for `lats`: it uses `lats[0]` and `lats[-1]` as the positions of row 0 and row H-1 in a linear interpolation, so ascending and descending both work.

1. **Mask.** `mask = data >= THRESHOLD_DBZ` (NaN compares false).
2. **Connected components.** `scipy.ndimage.label(mask)` with default structuring element, which is **4-connectivity** (von Neumann cross, not 8-connectivity). Labels `1..n` are numbered in row-major order of each component's first pixel. The Rust port must produce the same numbering or the tie order in step 8 differs.
3. **Empty.** If `n == 0`, return the empty shape (6.7).
4. **Per-component stats** (label `i`, `1..n`):
   - `size_i` = number of mask pixels (int64).
   - `peak_i` = maximum of `data` over the component's pixels (float32, then `float()`).
   - `(r̄, c̄)` = unweighted centroid of pixel indices `(row, col)`, floats.
   - Bounding slices `(rs, cs)` from `find_objects`: `rs = [r0, r1)`, `cs = [c0, c1)` (half-open).
5. **Normalization of longitudes.** `lons_norm = where(lons > 180, lons - 360, lons)`. `lat_top = lats[0]`, `lat_bottom = lats[-1]`, `lon_left = lons_norm[0]`, `lon_right = lons_norm[-1]` (floats).
6. **Cell size.** `dlat_km = |lats[1]-lats[0]| * 110.574` if `len(lats) > 1` else `1.0`. `dlon_deg = |lons_norm[1]-lons_norm[0]|` if `len > 1` else `0.01`. `cell_km2(lat) = dlat_km * dlon_deg * 111.320 * cos(radians(lat))`.
7. **Per-component features.** For each `i` in label order:
   - Skip if `size_i < MIN_PIXELS` (keep when `>= 5`).
   - `lat = lat_top + (lat_bottom - lat_top) * (r̄ / max(1, H-1))`.
   - `lon = lon_left + (lon_right - lon_left) * (c̄ / max(1, W-1))`.
   - `area_km2 = size_i * cell_km2(lat)` (note: uses the centroid's latitude).
   - `bbox = _component_bbox(rs, cs, lats, lons_norm)` (6.4).
   - Feature (keys in this order): `type: "Feature"`, `geometry: {type: "Point", coordinates: [lon, lat]}`, `properties: {peak_dbz, area_km2, pixel_count (int), bbox}`.
8. **Sort and cap.** `features.sort(key=peak_dbz, reverse=True)` (stable: ties keep label order). Then `features = features[:500]`.
9. **Tracking.** `_attach_tracking(features, previous, timestamp)` (6.5). It mutates each feature's `properties`.
10. **Output** (6.7).

### 6.4 Bounding box (`_component_bbox`)

Input: the bounding slices and `lats`, `lons_norm`.
1. `row_start = rs.start`; `row_stop = max(row_start, rs.stop - 1)`. Same for columns.
2. `lat_values = (lats[row_start], lats[row_stop])` (raw indices, **not** min/max over all pixels). `lon_values = (lons_norm[col_start], lons_norm[col_stop])`.
3. `south, north = min/max(lat_values)`; `west, east = min/max(lon_values)`.
4. `center_lat = (south + north) / 2`.
5. `lat_padding = 12 / 110.574` degrees. `lon_scale = max(1e-6, 111.320 * cos(radians(center_lat)))`. `lon_padding = 12 / lon_scale`.
6. Return `[west - lon_padding, south - lat_padding, east + lon_padding, north + lat_padding]`. **No rounding.** Format: `[west, south, east, north]`.

### 6.5 Tracking and association (`_attach_tracking`)

Inputs: `features` (current, sorted, capped), `previous` (the parsed previous `storms.json` dict, or `None`), `timestamp`.

Setup:
- `previous_features = previous["features"]` filtered to dicts (or `[]`).
- `current_dt = parse(timestamp)`; `previous_dt = parse(previous["timestamp"])`.
- `elapsed_minutes = (current_dt - previous_dt).total_seconds() / 60` if both parse, else `0.0`. **[HAZARD]** Mixed naive/aware raises `TypeError` (uncaught). See 14.
- `now_s = current_dt.timestamp()` if current parses (naive is interpreted in local time by Python), else `time.time()` (non-deterministic).
- `previous_ids` = integer `properties.cell_id` of each previous feature (entries where `int()` fails are skipped).
- `next_id = max(previous_ids, default=0) + 1`. **[DEFECT]** IDs are not monotonic. The counter is derived from the **previous frame's** max, so when the highest-numbered cell disappears, its ID is reused two frames later. See 14.
- `matches: dict[current_index -> previous_index] = {}`.

Association runs only when `features and previous_features and 0 < elapsed_minutes <= 15`.
1. `predicted[j]` for each previous feature `j`: `old_coords = (lon, lat)` from `old.geometry.coordinates`; `(east, north) = _previous_motion(old.properties)` (from `tracking_vector.east_kmh/north_kmh`, default `(0,0)`); `predicted[j] = _advance(old_coords, east, north, elapsed_minutes)`. On `KeyError`, `TypeError` or `ValueError`, `predicted[j] = None`.
2. Cost matrix `D` of shape `(n_current, n_previous)`, initialized to `1e9`. For each current `i` and each previous `j` with non-None prediction: `D[i, j] = haversine_km(current_coords_i, predicted_j)`. Note the `_haversine_km` argument form: `(lon, lat)` tuples.
3. `rows, cols = linear_sum_assignment(D)`: globally optimal one-to-one assignment minimizing total cost over `min(n_current, n_previous)` pairs. **The optimal global assignment is required; a greedy match gives different results.**
4. `base_gate = max(MATCH_SLACK_KM, MAX_TRACK_SPEED_KMH * elapsed_minutes / 60 * 0.5)` km. For `elapsed = 2 min` this is `max(8, 2.67) = 8`. For `elapsed = 10 min`, `max(8, 13.33) = 13.33`.
5. For each pair `(row, col)`: `area = features[row].properties.area_km2` (default 0). If `D[row, col] <= max(base_gate, 0.5 * sqrt(max(area, 0)))`: `matches[row] = col`. Pairs failing the gate are discarded.

Assignment loop, for each current feature `index` in order:
- **If matched**: `cell_id = int(old_props.get("cell_id", next_id))`. `history = [h for h in old_props.get("track_history") or [] if list and len(h)==3 and (now_s - float(h[0])) <= TRACK_HISTORY_MIN*60]`.
- **Else**: `cell_id = next_id`; `next_id += 1`; `history = []`. Note that `next_id` only exceeds previous IDs, so unmatched cells in this frame never reuse an ID that is still present in the previous frame. Reuse happens in a later frame (see D3).
- `history.append([round(now_s, 1), round(lon, 5), round(lat, 5)])` (lon and lat are the feature's coordinates).
- `props["cell_id"] = cell_id`; `props["track_history"] = history` (key order: `cell_id`, then `track_history`).
- `span_min = (history[-1][0] - history[0][0]) / 60`.
- If `len(history) >= 2 and span_min >= 5`:
  `(east_kmh, north_kmh, rms_km) = _fit_motion(history)`; `confidence = min(1.0, span_min / 15.0) / (1.0 + rms_km / 3.0)`.
- Else `east = north = 0.0`, `confidence = 0.0`.
- `speed = hypot(east, north)`. If `speed > 160`: `east = north = speed = 0.0`, `confidence = 0.0`.
- `bearing = ((degrees(atan2(east, north)) + 360) % 360) if speed else 0.0`. Bearing is clockwise from north.
- `props["tracking_vector"] = {east_kmh: round(east,3), north_kmh: round(north,3), speed_kmh: round(speed,3), bearing_deg: round(bearing,2)}`.
- `props["tracking_confidence"] = round(confidence, 2)`.
- `props["predicted_bboxes"] = [{"lead_minutes": L, "bbox": _translate_bbox(props["bbox"], center_lat=lat, east_kmh=east, north_kmh=north, lead_minutes=L)} for L in (0, 5, 10)]`. Uses **unrounded** `east`/`north`.

### 6.6 Motion fit (`_fit_motion`)

Input: `history` of `[epoch_s, lon, lat]`, the last element being the current sample.
1. `t = epoch_s / 3600` (hours), `lon`, `lat` as float64 arrays.
2. `x = (lon - lon[-1]) * 111.320 * cos(radians(lat[-1]))` (km east of last sample, scaled by the last latitude).
3. `y = (lat - lat[-1]) * 110.574` (km north).
4. `tc = t - mean(t)`; `denom = sum(tc*tc)`. If `denom <= 0`: return `(0, 0, 0)`.
5. `ve = sum(tc * (x - mean(x))) / denom` (km/h). `vn` likewise with `y`.
6. `resid = hypot(x - (mean(x) + ve*tc), y - (mean(y) + vn*tc))`. `rms = sqrt(mean(resid*resid))`.
7. Return `(ve, vn, rms)`.

Note the constants: `111.320` for longitude and `110.574` for latitude in this function.

### 6.7 Track-history retention

- History entries older than `TRACK_HISTORY_MIN * 60 = 1200` seconds relative to the new `now_s` are dropped. Only matched cells carry history forward. Unmatched cells start empty.
- The stored sample times are `round(now_s, 1)` (one decimal, epoch seconds). Coordinates are `round(., 5)`.

### 6.8 Association examples (from `test_storms.py`)

- Two frames, cell moves 1 pixel east in 5 min (`_grid(7)` then `_grid(8)`, timestamps 12:00 and 12:05): same `cell_id`, `east_kmh > 0`, predictions `[0, 5, 10]`, and `predicted_bboxes[2].bbox[0] > predicted_bboxes[0].bbox[0]`. `tracking_vectors` has length 1.
- 8 frames, 2 min apart, moving 60 km/h east with ±1 cell wobble: `len(track_history) == 8`, `speed_kmh in (45, 75)`, `bearing_deg in (75, 105)`, `tracking_confidence > 0.5`.
- 2-min gap with one 2-min step: same `cell_id`, but `speed_kmh == 0.0` and `tracking_confidence == 0.0` (span < 5 min).
- Cell jumps ~16 km in 2 min: **new** `cell_id` (gate is 8 km + half-width).
- 10x10 block: `area_km2` within 5% of `100 * (0.01*110.574) * (0.01*111.32*cos(42.05°))`.

### 6.9 `write_storms_json(state_dir, data, lats, lons, timestamp)`

Algorithm, under `storms.lock` (LOCK_EX):
1. `state_dir.mkdir(parents=True, exist_ok=True)`. Lock file `state_dir/storms.lock`. Path `state_dir/storms.json`.
2. `previous = json.loads(read_text())`, or `None` on `OSError` or `JSONDecodeError`. **Other exceptions propagate.**
3. If `current_dt` and `previous_dt` both parse and `current_dt <= previous_dt`: return **without writing** (equal timestamps are also skipped). Lock released by fd close.
4. `payload = detect_storms(data, lats, lons, timestamp=timestamp, previous=previous)`.
5. `mkstemp(prefix=".storms.", suffix=".tmp", dir=state_dir)`. Write style B (compact, unsorted, newline). `os.replace(tmp, path)`. `finally` unlink tmp ignoring `FileNotFoundError`. No fsync.
6. `LOCK_UN`.

Only the `radar` layer writes `storms.json` (`ingest_mrms/activities.py`, the `_grids` helper, around line 376). The composite layer deliberately does not, to avoid racing cell IDs.

### 6.10 Output JSON schema

Non-empty case (keys in this order):
```
{
  "type": "FeatureCollection",
  "features": [Feature...],
  "generated_at": <float epoch seconds, time.time() unrounded>,
  "timestamp": "<timestamp as passed>",
  "threshold_dbz": <float, e.g. 40.0>,
  "cell_count": <int = len(features)>,
  "prediction_minutes": [0, 5, 10],
  "tracking_vectors": [{"cell_id": int, "east_kmh": f, "north_kmh": f, "speed_kmh": f, "bearing_deg": f}, ...]
}
```

Feature:
```
{
  "type": "Feature",
  "geometry": {"type": "Point", "coordinates": [lon, lat]},
  "properties": {
    "peak_dbz": f, "area_km2": f, "pixel_count": int,
    "bbox": [west, south, east, north],          (unrounded, 6.4)
    "cell_id": int,
    "track_history": [[epoch_s_1dp, lon_5dp, lat_5dp], ...],
    "tracking_vector": {"east_kmh": f3, "north_kmh": f3, "speed_kmh": f3, "bearing_deg": f2},
    "tracking_confidence": f2,
    "predicted_bboxes": [
      {"lead_minutes": 0,  "bbox": [w6, s6, e6, n6]},
      {"lead_minutes": 5,  "bbox": [...]},
      {"lead_minutes": 10, "bbox": [...]}
    ]
  }
}
```

Empty case (`n == 0`), which is **a different shape**: keys are `type`, `features` (`[]`), `generated_at`, `timestamp`, `prediction_minutes`. **No** `threshold_dbz`, `cell_count` or `tracking_vectors`. [DEFECT] Consumers must tolerate both. See 14.

When `features` is empty after the size filter, the non-empty branch still runs (with `cell_count: 0` and `tracking_vectors: []`).

### 6.11 Example (abbreviated)

```
{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[-84.9,42.9]},"properties":{"peak_dbz":55.0,"area_km2":21.4,"pixel_count":25,"bbox":[-85.05,42.78,-84.75,43.02],"cell_id":1,"track_history":[[1783512000.0,-84.9,42.9]],"tracking_vector":{"east_kmh":0.0,"north_kmh":0.0,"speed_kmh":0.0,"bearing_deg":0.0},"tracking_confidence":0.0,"predicted_bboxes":[{"lead_minutes":0,"bbox":[-85.05,42.78,-84.75,43.02]},{"lead_minutes":5,"bbox":[-85.05,42.78,-84.75,43.02]},{"lead_minutes":10,"bbox":[-85.05,42.78,-84.75,43.02]}]}}],"generated_at":1783512000.123,"timestamp":"2026-07-10T12:00:00+00:00","threshold_dbz":40.0,"cell_count":1,"prediction_minutes":[0,5,10],"tracking_vectors":[{"cell_id":1,"east_kmh":0.0,"north_kmh":0.0,"speed_kmh":0.0,"bearing_deg":0.0}]}
```
(The bbox numbers are illustrative.)

---

## 7. Storm prefetch plan (`storm_prefetch.py`)

### 7.1 Constants

```
MAX_PREFETCH_DISTANCE_KM = float(env STORM_PREFETCH_MAX_DISTANCE_KM, 500)
MIN_ZOOM                 = 4
MAX_ZOOM                 = {"radar": 7, "nowcast": 6}
EARTH_RADIUS_KM          = 6371.0088
```

### 7.2 Public API

```
build_storm_prefetch_plan(*, storms: dict, state_dir, tile_dir, base_url: str,
                          lat: float, lon: float, zoom: int, palette: str) -> dict
```

Private: `_lat_lon_to_tile`, `_parse_timestamp`, `_distance_km(lat, lon, other_lat, other_lon)` (**note: lat-first argument order**, unlike storms `_haversine_km`), `_select_storm`, `_nearest_timestamp`, `_frame_for_lead`, `_tiles_for_bbox`, `_tile_path`.

### 7.3 Storm selection (`_select_storm`)

For each feature (skip on `KeyError`, `TypeError`, `ValueError`):
- `distances` = for each `predicted_bboxes[k]` with `bbox = [w, s, e, n]`: `_distance_km(lat, lon, (s+n)/2, (w+e)/2)` (distance from the user to each predicted bbox center).
- `distance = min(distances)`.
- `severity = max(0, float(peak_dbz default 40) - 40)`.
- `area = max(1, float(area_km2 default 1))`.
- `score = distance - min(35, severity*0.8 + log10(area)*3)`.

Sort ascending by score (stable). Take the first. If none: return `None`.
Then check the **geometry** point (`geometry.coordinates = [lon, lat]`): if `_distance_km(lat, lon, sel_lat, sel_lon) > 500`: return `None`. Note the score uses bbox centers and the gate uses the feature point.

### 7.4 Frame selection per lead (`_frame_for_lead`)

`anchor = parse(storms.timestamp)`. If `None`, the whole plan is empty (7.6).

- **Lead 0**: `radar` timestamps from the manifest. `_nearest_timestamp(radar, anchor, prefer_past=True)`: candidates with `parsed <= anchor`, minimum absolute difference, ties broken by list order (first wins). Returns `("radar", ts)` or `None`.
- **Other leads** (5 and 10): `target = anchor + lead minutes`. `_nearest_timestamp(nowcast, target)` (no past preference). If none, or `|parsed - target| > 8*60` seconds (strictly greater), return `None`. Otherwise `("nowcast", ts)`.

### 7.5 Tiles

- `_lat_lon_to_tile(lat, lon, z)`: `n = 2**z`. `x = int((lon+180)/360*n)`. `clipped = clamp(lat, -85.05112878, 85.05112878)`. `y = int((1 - asinh(tan(radians(clipped)))/pi) / 2 * n)`. Clamp `x, y` to `[0, n-1]`. `int()` truncates toward zero.
- `_tiles_for_bbox(bbox, z)`: `(x_min, y_min) = tile(north, west)`, `(x_max, y_max) = tile(south, east)`. Return all `(x, y)` with `x` in `[min, max]` inclusive and `y` in `[min, max]` inclusive. Row-major order: `x` outer, `y` inner.
- `frame_zoom = min(requested_zoom, MAX_ZOOM[layer])`. `requested_zoom = max(4, int(zoom))`.

### 7.6 Tile URL and existence

`_tile_path(layer, palette, timestamp, zoom, x, y)`:
```
"/" + "/".join(quote(part, safe=":+-.T_") for part in
  ("tiles", layer, palette, timestamp, str(zoom), str(x), f"{y}.png"))
```
Example: `/tiles/radar/classic/2026-07-10T12:00:00+00:00/6/33/22.png`. Characters `:` and `+` are left unescaped.

**[DEFECT]** This uses the **timestamp** as the path segment. The manifest's own frame `path` is the authoritative tile path (see 3.3 and the `tile_url_template`). For nowcast frames `path` is `runs/<anchor>/<valid>`, and tiles are written under that path (`nowcast/activities.py` `_render_frame`, `shutil.rmtree(TILE_DIR/"nowcast"/palette/tile_path)`). So for leads 5 and 10 this function builds a path that does not exist on disk and drops every URL silently (`disk_path.is_file()` is false). The test passes only because its fixture writes files at the timestamp path. See 14.

Disk check: `disk_path = Path(tile_dir) / relative.removeprefix("/tiles/")`. Add `f"{base_url}{relative}"` only when `disk_path.is_file()`.

### 7.7 Plan assembly (`build_storm_prefetch_plan`)

1. `selected = _select_storm(...)`; `anchor = parse(storms["timestamp"])`. If either is `None`: return `{"plan_id": None, "storm_cell_id": None, "bboxes": [], "tile_urls": []}`.
2. `manifest = read_manifest_file(state_dir)`. **This can raise** on a corrupt manifest (3.4). The caller must handle it.
3. `base_url = base_url.rstrip("/")`.
4. For each of the first 3 `predicted_bboxes` (`[:3]`):
   - `lead = int(lead_minutes)`; `bbox = [float(v) for v in bbox]`.
   - `frame = _frame_for_lead(manifest, anchor, lead)`.
   - If `None`: append `{"lead_minutes", "bbox", "layer": null, "timestamp": null, "zoom": requested_zoom, "style_url": null, "tile_urls": []}`. Continue.
   - Else: `frame_zoom`; `urls` = existing tile URLs (7.6); `query = urlencode({"layer", "palette", "timestamp", "zoom": frame_zoom})` (quote_plus encoding: `:` -> `%3A`, `+` -> `%2B`); append `{"lead_minutes", "bbox", "layer", "timestamp", "zoom": frame_zoom, "style_url": f"{base_url}/api/storm-prefetch/style.json?{query}", "tile_urls": urls}`. Extend `all_urls`.
5. `cell_id = properties["cell_id"]`.
6. Return:
```
{
  "plan_id": f"{storms['timestamp']}:{cell_id}:{palette}:{requested_zoom}",
  "storm_cell_id": cell_id,
  "generated_at": storms.get("generated_at"),
  "tracking_vector": properties.get("tracking_vector"),
  "bboxes": [...],            (exactly as built above; length 3 when selection succeeds)
  "tile_urls": [...]          (deduplicated, first-occurrence order)
}
```
Note: `plan_id` uses `requested_zoom`, not the per-frame zoom.

Example of a bbox entry:
```
{"lead_minutes":5,"bbox":[-85.12,42.71,-84.62,43.08],"layer":"nowcast","timestamp":"2026-07-10T12:05:00+00:00","zoom":6,"style_url":"https://radar.example/api/storm-prefetch/style.json?layer=nowcast&palette=classic&timestamp=2026-07-10T12%3A05%3A00%2B00%3A00&zoom=6","tile_urls":["https://radar.example/tiles/nowcast/classic/2026-07-10T12:05:00+00:00/6/33/22.png"]}
```

### 7.8 Test invariants (`test_storms.py`)

- Plan has exactly three `bboxes`, leads `[0, 5, 10]`; `tile_urls` non-empty; every URL starts with `base_url + "/tiles/"`; every bbox has a non-empty `style_url`. The manifest in that test uses a template with `{timestamp}`; the fixture writes tiles at `tiles/<layer>/classic/<timestamp>/6/<x>/<y>.png`.

---

## 8. Storm watch (`storm_watch.py`)

### 8.1 Constants

```
GRID_DIR             = Path(env GRID_DIR, "/data/grids")
WATCH_RADIUS_KM      = float(env WATCH_RADIUS_KM, 20)     (callers pass radius explicitly)
INTENSIFY_DBZ_DELTA  = float(env INTENSIFY_DBZ_DELTA, 10)
DISSIPATE_DBZ_DELTA  = float(env DISSIPATE_DBZ_DELTA, 10)
HAIL_DBZ_THRESHOLD   = float(env HAIL_DBZ_THRESHOLD, 60)
```
Latitude km/degree: **111.32** (`_km_per_deg_lat`). Longitude: `111.32 * cos(lat)`. This differs from storms (110.574). See 14.

### 8.2 Data classes

- `FrameSample(timestamp: str, max_dbz: float, mean_dbz: float, above_50_count: int)`
- `FrameDiff(prev: FrameSample | None, curr: FrameSample, max_dbz_delta: float)`
- `ChangeKind(kind: str, summary: str)`. Kinds produced by this module: `severe`, `intensifying`, `dissipating`. `appeared` and `disappeared` are declared but never produced.

### 8.3 `latest_radar_meta() -> Path | None`

- `GRID_DIR/"radar"` must exist. `sorted(glob("*.meta.json"))`, return the last element, or `None`.
- Sorting is by file name, so the last is the largest timestamp (ISO strings).

### 8.4 `sample_window(meta_path, lat, lon, radius_km) -> FrameSample | None`

`None` (not an exception) when:
- meta JSON cannot be read or parsed; `height`, `width`, `lat_min`, `lat_max`, `lon_min`, `lon_max` missing; `ValueError` from reshape; `OSError` reading the binary.

Steps:
1. `h, w` from meta. `data_file` from meta, or `meta_path.name.replace(".meta.json", ".bin")` if missing or empty. `bin_path = meta_path.parent / data_file`.
2. `arr = np.fromfile(bin_path, dtype="<f4").reshape(h, w)`.
3. `lat_max, lat_min, lon_min, lon_max` floats from meta.
4. **Out of bounds** (`not (lat_min <= lat <= lat_max and lon_min <= lon <= lon_max)`, inclusive): return `FrameSample(timestamp=name_without_suffix, max_dbz=-inf, mean_dbz=nan, above_50_count=0)`. This is a **sample**, not `None`.
5. `row_center = round((lat_max - lat) / (lat_max - lat_min) * (h - 1))`. `col_center = round((lon - lon_min) / (lon_max - lon_min) * (w - 1))`. **`round` is Python's round-half-to-even.**
6. `half_lat_deg = radius_km / 111.32`. `half_lon_deg = radius_km / (111.32 * cos(radians(lat)))`.
7. `half_rows = max(1, round(half_lat_deg / (lat_max - lat_min) * (h - 1)))`. `half_cols = max(1, round(half_lon_deg / (lon_max - lon_min) * (w - 1)))`.
8. `r0 = max(0, rc - half_rows)`, `r1 = min(h, rc + half_rows + 1)`, `c0 = max(0, cc - half_cols)`, `c1 = min(w, cc + half_cols + 1)`. Window `arr[r0:r1, c0:c1]` (half-open).
9. `win = win[win > -100]` (drops fill values, MRMS -999/-99 and our -9999, and NaN).
10. If empty: `FrameSample(..., max_dbz=-inf, mean_dbz=nan, above_50_count=0)`.
11. Else `max_dbz = float(win.max())`, `mean_dbz = float(win.mean())` (numpy float32 mean), `above_50_count = count(win >= 50)`.

Note: `float32` mean precision. Numpy reduces float32 with pairwise summation in float32, so Rust should use pairwise f32 or accept a small tolerance (14).

### 8.5 `detect_change(diff: FrameDiff) -> ChangeKind | None`

In order:
1. If `diff.prev is None`: return `None`.
2. If `curr.max_dbz >= 60`: `ChangeKind("severe", f"reflectivity {curr.max_dbz:.0f} dBZ — possible hail/severe")`. Note the em dash.
3. If `diff.max_dbz_delta >= 10`: `ChangeKind("intensifying", f"+{delta:.0f} dBZ since last frame ({prev.max_dbz:.0f}→{curr.max_dbz:.0f})")`. Note the arrow `→`.
4. If `-diff.max_dbz_delta >= 10`: `ChangeKind("dissipating", f"{delta:.0f} dBZ since last frame ({prev.max_dbz:.0f}→{curr.max_dbz:.0f})")`. The delta carries its own sign, e.g. `-12 dBZ ...`.
5. Else `None`.

`max_dbz_delta` is computed by the **caller** (`api/api/storm_watch_activities.py:compare_radar_frames`): `delta = sample.max_dbz - prev_max_dbz` (0.0 when no previous). See 8.6 and 14 for the `-inf` consequence.

### 8.6 Caller contract (for context)

`compare_radar_frames` (`storm_watch_activities.py:94`): `sample_window(meta, lat, lng, radius_km=20.0)`. Returns `sampled=False` when no meta, `None` sample, or `prev_timestamp == sample.timestamp`. `curr_max_dbz` is `-99.0` when the sample is `-inf`. `max_dbz_delta` is computed from the **unclamped** `sample.max_dbz`.

`detect_storm_change` (`storm_watch_activities.py`): with `has_prev == False` returns empty kind/summary. Otherwise builds a `FrameDiff` (with `mean_dbz = nan` and `above_50_count = 0`, which `detect_change` ignores) and calls `detect_change`.

---

## 9. Push tokens (`push_tokens.py`)

### 9.1 Storage

- File: `STATE_DIR/push_tokens.sqlite`. `STATE_DIR` is a `Path` (`env STATE_DIR`, default `/data/state`). Evaluated at import.
- Every operation opens a **new connection**: `sqlite3.connect(str(DB_PATH))` with Python defaults (timeout 5.0 s, deferred transactions, no WAL, default rollback journal). `STATE_DIR.mkdir(parents=True, exist_ok=True)` first.
- After opening: `executescript(schema)` (idempotent). `yield conn`. On normal exit `conn.commit()`. `finally conn.close()`. On exception inside the body, `commit` is skipped and close rolls back.

### 9.2 Schema (exact)

```sql
CREATE TABLE IF NOT EXISTS push_tokens (
    user_id    TEXT NOT NULL,
    token      TEXT NOT NULL PRIMARY KEY,
    platform   TEXT NOT NULL,
    created_at REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_user ON push_tokens(user_id);
```

**[DEFECT / HAZARD]** The module docstring says tokens are "unique per user + platform". The schema makes `token` the **global** primary key. Registering the same token for a different user **reassigns** it (`ON CONFLICT(token) DO UPDATE SET user_id=excluded.user_id, platform=excluded.platform`), and `created_at` keeps the original value.

### 9.3 Type

```
@dataclass PushToken: user_id: str, token: str, platform: str  # "ios" | "android" (not validated), created_at: float
```

### 9.4 Functions

| Function | SQL | Returns |
|---|---|---|
| `upsert(user_id, token, platform) -> None` | `INSERT INTO push_tokens(user_id, token, platform, created_at) VALUES(?,?,?,?) ON CONFLICT(token) DO UPDATE SET user_id=excluded.user_id, platform=excluded.platform`, with `created_at = time.time()` (float seconds, unrounded) | nothing |
| `delete_by_token(token) -> int` | `DELETE FROM push_tokens WHERE token = ?` | `rowcount` |
| `delete_for_user(user_id, token) -> int` | `DELETE ... WHERE user_id = ? AND token = ?` | `rowcount` |
| `list_for_user(user_id) -> list[PushToken]` | `SELECT user_id, token, platform, created_at FROM push_tokens WHERE user_id = ?` | list; **no ORDER BY** (order is unspecified; SQLite will likely return `idx_user` order, but do not depend on it) |

No validation of `platform` or `token` contents.

---

## 10. Nowcast skill log (`nowcast_skill.py`)

### 10.1 Constants

```
SKILL_FILE            = "nowcast-skill.json"
SCHEMA_VERSION        = 1
THRESHOLDS_DBZ        = {"rain": 20.0, "heavy": 35.0}          (dict order: rain, heavy)
WINDOW_HOURS          = int(env NOWCAST_SKILL_WINDOW_HOURS, 24)
HEADLINE_LEAD_MIN     = int(env NOWCAST_SKILL_HEADLINE_LEAD_MIN, 30)
MIN_RUNS_FOR_HEADLINE = int(env NOWCAST_SKILL_MIN_RUNS, 6)
```

### 10.2 File: `<state_dir>/nowcast-skill.json` (writer style A)

Top-level keys (sorted): `entries, schema_version, thresholds_dbz, updated_at, window_hours`.

```
{"entries":[ENTRY...],"schema_version":1,"thresholds_dbz":{"heavy":35.0,"rain":20.0},"updated_at":"2026-09-30T12:00:00+00:00","window_hours":24}
```

Each `ENTRY` (written by `nowcast/skill.py` `score_observation`, passed through unchanged):

| Key | Type | Meaning |
|---|---|---|
| `anchor` | string | run id (run directory name, ISO) |
| `valid` | string | observed valid time being scored (ISO) |
| `lead_minutes` | int | `round((valid_dt - anchor_dt) / 60 s)` (Python banker's round) |
| `scored_at` | string | `now.isoformat()` (UTC-aware) |
| `cells` | int | count of "known" observed cells (same for every threshold) |
| `thresholds` | object `{rain: C, heavy: C}` | `C = {hits, misses, false_alarms}` (ints) |
| `persistence` | object `{rain: C, heavy: C}` **or `null`** | same counts for the "observed anchor, no motion" baseline. `null` when the baseline is missing |

Note: `cells` is not inside `thresholds`. The test fixture `_entry` also includes `cells` and `valid`.

`read_log` does not validate entries beyond `entries` being a list (see 14).

### 10.3 Empty log

`{"schema_version": 1, "window_hours": WINDOW_HOURS, "thresholds_dbz": {"rain": 20.0, "heavy": 35.0}, "updated_at": null, "entries": []}`.

### 10.4 Functions

**`skill_path(state_dir) -> Path`** = `state_dir / "nowcast-skill.json"`.

**`read_log(state_dir) -> dict`**: reads the file. `OSError` or `ValueError` (incl. JSONDecodeError) -> `empty_log()`. Body not a dict, or `entries` not a list -> `empty_log()`. Otherwise returns the body as-is (no field normalization).

**`_parse_ts(value) -> datetime | None`**: `fromisoformat(str(value).replace("Z", "+00:00"))`, `TypeError`/`ValueError` -> `None`. Naive -> UTC (`dt.replace(tzinfo=timezone.utc)`).

**`append_entries(state_dir, entries, *, now=None, window_hours=None) -> dict`**
1. `now = now or utcnow`. `window = timedelta(hours = window_hours or WINDOW_HOURS)`.
2. `log = read_log(state_dir)` (a corrupt file is replaced by an empty log, silently).
3. `kept = [e for e in [*log.entries, *entries] if (scored := _parse_ts(e.scored_at)) is not None and now - scored <= window]`. Entries with unparseable `scored_at` are dropped. Entries **in the future** are kept (negative age).
4. `kept.sort(key=(str(scored_at), int(lead_minutes or 0)))`. The sort key is the **string** of `scored_at`.
5. `log.update(schema_version=1, window_hours = window_hours or WINDOW_HOURS, thresholds_dbz = {...}, updated_at = now.isoformat(), entries = kept)`. Other top-level keys already in the file are kept.
6. Write style A to `nowcast-skill.json` via `mkstemp(prefix=".nowcast-skill.json.", suffix=".tmp")` then `os.replace`. No fsync. **No lock.**
7. Return `log`.

Concurrency: read-modify-write with no lock. The only writer is the nowcast worker (`nowcast/skill.py:172`), which runs under one Temporal activity at a time per anchor. Concurrent writers would lose entries.

**`_ratio(num, den) -> float | None`**: `round(num / den, 3)` if `den > 0` else `None`. Python `round` (half-even on the binary value).

**`scores_from_counts(counts) -> dict`**: `hits, misses, false_alarms` (`int(get(k, 0))`).
```
pod  = _ratio(hits, hits + misses)
far  = _ratio(false_alarms, hits + false_alarms)
csi  = _ratio(hits, hits + misses + false_alarms)
bias = _ratio(hits + false_alarms, hits + misses)
```
Returns `{hits, misses, false_alarms, pod, far, csi, bias}`.

**`_add_counts(into, counts)`**: no-op if `counts` is not a dict. For `hits`, `misses`, `false_alarms`: `into[k] = into.get(k,0) + int(counts.get(k,0) or 0)`.

**`summarize(log, *, now=None, window_hours=None) -> dict`**
1. `now` default UTC now. `window = timedelta(hours = window_hours or int(log.get("window_hours") or WINDOW_HOURS))`.
2. `by_lead: dict[int, bucket]`, `anchors: set[str]`.
3. For each entry (skip non-dicts):
   - `scored = _parse_ts(scored_at)`. Skip if `None` or `now - scored > window`. (Entries **in the future** are kept.)
   - `lead = int(entry["lead_minutes"])`. Skip on `KeyError`, `TypeError`, `ValueError`.
   - bucket `{n, anchors: set, counts: {}, persistence: {}}`. `bucket.anchors.add(str(anchor))`; `anchors.add(str(anchor))`.
   - `thresholds = entry.get("thresholds") or {}`, `persistence = entry.get("persistence") or {}`. For each label in `[rain, heavy]`: `_add_counts(bucket.counts[label], thresholds.get(label))` and the same for persistence.
4. `leads = []`. For each lead in ascending order: `row = {"lead_minutes": lead, "runs": len(bucket.anchors)}`. For each label: `scores = scores_from_counts(bucket.counts[label])`; `p = scores_from_counts(bucket.persistence[label])`; `scores["persistence_csi"] = p.csi if (p.hits + p.misses) else None`; `row[label] = scores`. The `runs` value counts **distinct anchors** at that lead.
5. `summary = {available: False, window_hours: int(window.total_seconds() // 3600), thresholds_dbz: {rain: 20.0, heavy: 35.0}, runs_scored: len(anchors) (distinct across leads), updated_at: log.updated_at, by_lead: leads, headline: None}`.
6. If `leads` is empty: `summary["reason"] = "no_scored_runs"`, return.
7. `headline_row = min(leads, key=(abs(lead - HEADLINE_LEAD_MIN), lead))`. The lead nearest 30 wins, with ties going to the smaller lead.
8. If `headline_row.runs < MIN_RUNS_FOR_HEADLINE` (6): `reason = "warming_up"`, return.
9. `rain = headline_row.rain`. If `rain.pod is None` (no observed rain): `reason = "no_rain_observed"`, return.
10. `available = True`. `headline = {lead_minutes, window_hours, runs, pod, far, csi, persistence_csi}` from `headline_row` and `rain`.
11. Return. No `reason` key when available.

**`prometheus_lines(summary) -> list[str]`**
```
# TYPE radar_ng_nowcast_skill_runs gauge
radar_ng_nowcast_skill_runs <int(runs_scored)>
```
If `by_lead` non-empty: for metric in `pod`, `far`, `csi`: line `# TYPE radar_ng_nowcast_<metric> gauge`, then for each row, for each label in `[rain, heavy]`: if the value is not `None`:
`radar_ng_nowcast_<metric>{lead_minutes="<lead>",threshold="<label>"} <value>`, with `<value>` as Python `str(float)` (for example `0.833`, `1.0`, `0.0`).

Note: `bias`, `persistence_csi` and counts are not exported.

### 10.5 Worked example (from tests)

- 8 entries at lead 30, `rain` counts `(hits 10, misses 2, fa 3)`, `persistence (6,6,6)`; 8 entries at lead 5 with `(20, 0, 0)`:
  - `runs_scored == 8`; `by_lead[5].rain.pod == 1.0`, `far == 0.0`.
  - `by_lead[30].rain.hits == 80`.
  - `headline`: `lead_minutes 30`, `runs 8`, `pod = round(80/96, 3)`, `far = round(24/104, 3)`, `persistence_csi = round(48/144, 3)`.
- 8 entries at lead 30 with zero counts: `reason == "no_rain_observed"`.
- 5 entries: `reason == "warming_up"`, `by_lead[0].runs == 5`.
- Prometheus: `radar_ng_nowcast_skill_runs 8`; `radar_ng_nowcast_pod{lead_minutes="15",threshold="rain"} 0.833`.
- `scores_from_counts({"hits":8,"misses":2,"false_alarms":2})`: `pod == 0.8`, `far == 0.2`, `csi == round(8/12,3)`, `bias == 1.0`. Empty counts: `pod`, `far`, `csi` are `None`.
- `append_entries` with `now = 2026-09-30T12:00Z`: entry 30 h old is dropped, 5-min-old kept. `updated_at == now.isoformat()`. Directory listing after the write contains only `nowcast-skill.json`.
- `read_log` on `{nope` or `[]` returns `entries == []`.

---

## 11. Logger (`logger.py`)

### 11.1 Line format (`JsonFormatter.format`)

One JSON object per record, written to stdout (`StreamHandler(sys.stdout)`), using `json.dumps(payload, default=str)` (writer style D: `", "` and `": "` separators, insertion order, `ensure_ascii` on, no newline inside).

Payload keys, in insertion order:
1. `"ts"`: `self.formatTime(record, "%Y-%m-%dT%H:%M:%S%z")`. Local time from `time.localtime` (the logging default converter), second precision, offset **without a colon** (`+0000`), e.g. `2026-10-13T12:00:00+0000`.
2. `"level"`: `record.levelname` (`DEBUG`, `INFO`, `WARNING`, `ERROR`, `CRITICAL`).
3. `"service"`: `record.name` (the logger name passed to `get_logger`).
4. `"msg"`: `record.getMessage()`.
5. `"exc"`: only if `record.exc_info`: `formatException(exc_info)` (traceback text).
6. Then every other attribute in `record.__dict__` that: does not start with `_`, and is not one of `args, asctime, created, exc_info, exc_text, filename, funcName, levelname, levelno, lineno, message, module, msecs, msg, name, pathname, process, processName, relativeCreated, stack_info, thread, threadName, taskName`. These come from the `extra=` dict, in the order given. **[DEFECT]** An `extra` key `ts`, `level` or `service` overwrites the base field silently. See 14.

Non-JSON values are stringified by `default=str`. NaN and Infinity are emitted as bare tokens.

Example line:
```
{"ts": "2026-10-13T12:00:00+0000", "level": "WARNING", "service": "radar-ingest", "msg": "retry", "fn": "fetch_grib", "attempt": 1, "max_attempts": 4, "delay": 1.0, "err": "connection reset"}
```
(Real output uses `, ` and `: ` separators, so the Rust output should match that spacing if byte compatibility is needed.)

### 11.2 `get_logger(service) -> logging.Logger`

- `logging.getLogger(service)`. If the attribute `_radar_ng_configured` is already set, return it unchanged.
- Otherwise: `StreamHandler(sys.stdout)` with `JsonFormatter`; `handlers.clear()`; add handler; `setLevel(env LOG_LEVEL, default "INFO")`; `propagate = False`; set `_radar_ng_configured = True`.
- Note: `setLevel` with an unknown name raises `ValueError`.

### 11.3 `retry(*, attempts=4, base_delay=1.0, max_delay=30.0, exceptions=(Exception,), log=None)`

Decorator for synchronous functions:
1. `delay = base_delay`; `last_exc = None`.
2. For `attempt` in `1..attempts`:
   - `return fn(*args, **kwargs)`.
   - On a matching exception: `last_exc = exc`. If `attempt == attempts`: break. If `log` given: `log.warning("retry", extra={"fn": fn.__name__, "attempt": attempt, "max_attempts": attempts, "delay": delay, "err": str(exc)})`. Then `time.sleep(delay)`. Then `delay = min(delay*2, max_delay)`.
   - Non-matching exceptions propagate immediately (no sleep, no log).
3. After the loop: `raise last_exc`.

Sleep schedule with defaults: 1.0, 2.0, 4.0 seconds (three sleeps, four attempts, seven seconds total), before the final raise. Each warning is emitted before a sleep, so the final failure is **not** logged as a warning.

---

## 12. Activity heartbeat (`activity_heartbeat.py`)

### 12.1 API

```
async def run_sync_with_heartbeat(func: Callable[..., T], *args: object,
                                  heartbeat_every: float,
                                  heartbeat_details: dict | Callable[[], dict]) -> T
```

### 12.2 Semantics

1. `work = asyncio.create_task(asyncio.to_thread(func, *args))`. The blocking callable runs in the default thread pool.
2. `beat` task: `while not work.done(): await sleep(heartbeat_every); if not work.done(): details = heartbeat_details() if callable else heartbeat_details; activity.heartbeat(details)`.
   - The first heartbeat fires after one full `heartbeat_every` interval, not at t=0.
   - The check after `sleep` prevents a beat after completion.
   - `heartbeat_details` is re-evaluated for each beat when callable.
3. `return await work`. `finally`: `beat.cancel()`; `with suppress(CancelledError): await beat`.

Failure behavior:
- If `func` raises, that exception propagates after `beat` is cancelled.
- If the outer activity is cancelled during `await work`, `CancelledError` propagates. **The thread keeps running** (threads are not cancellable); the function's result is discarded.
- **[HAZARD]** If `activity.heartbeat(...)` raises (for example, called outside an activity context), the beat task finishes with an exception. `await beat` in `finally` then raises that exception, which masks a successful result or replaces `func`'s exception.

### 12.3 Test invariant

`func` sleeps 50 ms, `heartbeat_every = 0.01`, details `{"phase": "test"}`. Result is `"done"`; at least one heartbeat is recorded; the first recorded value equals `{"phase": "test"}`.

---

## 13. Test invariant checklist

Each ID is a Rust test to write. Ties to the Python test in parentheses.

### 13.1 Manifest (`test_manifest.py`)
- **MAN-01** Add two frames to a new layer: `timestamps` ascending; `latest` equals the last. (`test_update_manifest_file_adds_and_removes_timestamp`)
- **MAN-02** Layer `palettes` is the intersection (`[]` when frames disagree); per-frame `palettes` preserved. Same test.
- **MAN-03** Remove a frame: `timestamps` and `latest` update. Same test.
- **MAN-04** `replace_layer_manifest` swaps the whole list, sorts it, sets `latest` and layer `palettes` from explicit `palettes`, and leaves other layers byte-equivalent in content. (`test_replace_layer_manifest_swaps_all_timestamps`)
- **MAN-05** `replace_layer_manifest(name, [])` removes the layer. (`test_replace_layer_manifest_empty_list_drops_layer`)
- **MAN-06** Read of a missing file yields `layers == {}`. (`test_read_manifest_file_missing_is_empty`)
- **MAN-07** Corrupt JSON: read raises a parse error; update raises; file content is unchanged. (`test_read_manifest_file_corrupt_raises_instead_of_wiping`)
- **MAN-08** Path is a directory: read raises an I/O error. (`test_read_manifest_file_unreadable_raises`)
- **MAN-09** (Derived, not in tests) A parsed body without `layers` dict yields an empty manifest on read. Port must decide (14).
- **MAN-10** (Derived) v1 document (no `frames`) is migrated to frames with `path == timestamp`.

### 13.2 Grid write (`test_grid_dump_atomic.py`)
- **GRD-01** Second `write_grid` with the same timestamp creates a new `.bin` name; the meta's `data_file` points to it; the old `.bin` still exists. (`test_grid_metadata_atomically_points_to_versioned_binary`)
- **GRD-02** With `max_cells=25` on a 10x10 grid, stored `height*width <= 25`. Same test.
- **GRD-03** First write for a new layer fsyncs both `GRID_DIR` and `GRID_DIR/<layer>`. (`test_first_layer_creation_syncs_its_parent`)
- **GRD-04** `_fsync_dir` propagates `EIO`. (`test_directory_fsync_eio_is_not_suppressed`)
- **GRD-05** `_fsync_dir` ignores `EINVAL` on fsync. (`test_directory_fsync_unsupported_error_is_ignored`)
- **GRD-06** A parent-directory fsync failure during `write_grid` propagates. (`test_write_grid_propagates_initial_parent_fsync_failure`)

### 13.3 Grid retention (`test_grid_dump_prune.py`)
- **GRD-07** `prune_grid_layer(keep=4)` on 8 stamps: removes 8 files, leaves the last 4 metas and their 4 bins; bins' timestamps equal the kept set. (`test_prune_keeps_newest_n_with_their_data_files`)
- **GRD-08** Old orphan `.bin` and legacy `{ts}.bin` removed; in-flight `.<ts>.meta.json.<rand>.tmp` kept; removed count 2. (`test_prune_removes_orphan_data_files_and_keeps_in_flight_tmp`)
- **GRD-09** Unconfigured layer: `prune_grid_layer("radar")` returns 0 and keeps all. (`test_prune_is_noop_for_unconfigured_layer`)
- **GRD-10** `keep=0` removes all committed (6 for 3 stamps) and keeps `.grid.lock`. (`test_prune_keep_zero_removes_all_committed_generations`)
- **GRD-11** Prune blocks while a commit (`os.replace` of meta) is in progress: prune does not enter until the commit finishes; then it returns 0 (the new generation is kept). (`test_prune_waits_for_grid_metadata_commit`)
- **GRD-12** `cleanup_old_grids` applies keep-N first: nowcast keeps 2 of 5 (removed 6); `radar` keeps 5 (young). (`test_cleanup_old_grids_applies_keep_n_before_age_sweep`)
- **GRD-13** Default `GRID_KEEP_LAST["radar-nowcast-input"] >= 5`. (`test_default_keep_covers_nowcast_inputs_plus_slack`) Value is 12.

### 13.4 Storms (`test_storms.py`)
- **STM-01** Tracking across frames: same `cell_id`; `east_kmh > 0`; `lead_minutes == [0,5,10]`; bbox at +10 is east of bbox at 0; `len(tracking_vectors) == 1`. (`test_detect_storms_tracks_cell_and_emits_three_predicted_bboxes`)
- **STM-02** `write_storms_json` does not regress: writing 12:05 then 12:00 leaves `timestamp == 12:05`. (`test_write_storms_json_does_not_regress_to_backfilled_frame`)
- **STM-03** Prefetch plan: 3 bboxes, leads `[0,5,10]`, non-empty `tile_urls`, all with the base URL prefix, all with `style_url`. (`test_prefetch_plan_has_exactly_three_bboxes_and_existing_tile_urls`)
- **STM-04** `_tiles_for_bbox` returns the right tile set (exercised via STM-03).
- **STM-05** Motion fitted over 8 frames: speed in (45,75), bearing in (75,105), confidence > 0.5, history length 8. (`test_motion_is_fitted_over_history_not_one_noisy_step`)
- **STM-06** Gap < 5 min span: same ID, zero speed, zero confidence. (`test_new_cells_report_no_motion_until_history_spans_enough_time`)
- **STM-07** Distant (16 km / 2 min) cell gets a new ID. (`test_distant_cell_is_not_associated_as_impossible_motion`)
- **STM-08** Area within 5% of the closed-form value for a 10x10 block. (`test_area_uses_the_source_grid_cell_size`)

### 13.5 Activity heartbeat
- **HBT-01** Beats while work runs; first beat details equal `{"phase": "test"}`; result returned. (`test_run_sync_with_heartbeat_beats_while_thread_runs`)

### 13.6 Nowcast skill
- **NSK-01** `scores_from_counts` classic formulas, bias, and `None` for empty denominators. (`test_scores_from_counts_is_the_classic_contingency_table`)
- **NSK-02** `append_entries` keeps 24 h window; leaves exactly one file in the directory; updated_at equals `now.isoformat()`; sorted order by `scored_at` and lead. (`test_append_keeps_only_the_window_and_writes_atomically`)
- **NSK-03** Unreadable or non-object log gives empty entries. (`test_unreadable_log_is_empty`)
- **NSK-04** Headline withheld with fewer than 6 runs (`warming_up`). (`test_summary_withholds_a_headline_until_enough_runs_scored`)
- **NSK-05** Per-lead aggregation, headline at 30 min, exact PODs/FARs/CSIs, `persistence_csi`. (`test_summary_aggregates_per_lead_and_picks_the_30_minute_headline`)
- **NSK-06** `no_rain_observed` when rain hits+misses = 0. (`test_summary_reports_no_rain_when_nothing_was_observed`)
- **NSK-07** Prometheus lines contain runs gauge and POD with `0.833`. (`test_prometheus_lines_expose_each_lead_and_threshold`)

### 13.7 Derived invariants (no direct Python test; port must keep)
- **DRV-01** `write_grid` stores `lats` descending (north first) and row 0 is north.
- **DRV-02** Longitudes > 180 are shifted and columns reordered ascending.
- **DRV-03** Stride is a power of two; `(h//s)*(w//s) <= cap`.
- **DRV-04** NaN -> -9999.0 only when `fill` is NaN.
- **DRV-05** `storms.json` is never replaced by an older or equal timestamp.
- **DRV-06** Storm IDs: matched cells keep ID; unmatched get `max(prev)+1`.
- **DRV-07** `detect_change`: `severe` takes precedence over `intensifying` and `dissipating`.
- **DRV-08** `sample_window`: out-of-bounds and all-fill windows return `max_dbz = -inf`, never `None`.
- **DRV-09** Manifest `timestamps` are sorted and unique after every write.
- **DRV-10** Nowcast log drops entries older than the window on every append.

---

## 14. Defects, hazards and inconsistencies (decision list)

Each item states what the code does, why it matters, and a recommended default. The default is "replicate for compatibility" unless marked "fix".

| # | Where | Behavior | Impact | Recommendation |
|---|---|---|---|---|
| D1 | `storm_prefetch._tile_path` (7.6) | Builds nowcast tile paths from `timestamp`; manifest and disk use `runs/<anchor>/<valid>`. | Leads 5 and 10 yield no tile URLs in production, silently. Fixture hides it. | **Fix.** Use the manifest frame `path` (and `tile_url_template`). Add a fixture with `runs/` paths. Confirm against a live manifest first. |
| D2 | `storm_watch` + `storm_watch_activities.compare_radar_frames` (8.4, 8.6) | `sample_window` returns `max_dbz=-inf` for out-of-bounds or all-fill windows. `max_dbz_delta = -inf - prev`. `detect_change` then reports `dissipating` with summary `-inf dBZ ...`. The API stores `curr_max_dbz = -99.0` in place of `-inf`; if the workflow feeds that back as `prev_max_dbz` (not traced in this pass), coverage returning gives a large false `intensifying` alert (for example `+129 dBZ` from `-99` to `30`). | Spurious "dissipating" or "intensifying" alerts when the watched point leaves coverage or is in a fill-only window. | **Fix** (skip the comparison when either sample is `-inf` or the previous value is a sentinel) or **replicate** and document. |
| D3 | `storms._attach_tracking` (6.5) | `next_id = max(prev IDs) + 1` from the previous frame only. | By reading the code: IDs can be reused. Example: frame N has `{1,2,3}`, frame N+1 has `{1,2}` (cell 3 gone), then a new cell in frame N+2 gets `3`. Any consumer keying on `cell_id` across frames can confuse the two cells. | **Fix** with a persisted monotonic counter (for example `next_id` stored in `storms.json`), or replicate and document. |
| D4 | `storm_watch._km_per_deg_lat` (8.1) vs storms/storm_prefetch (6.1) | Latitude km/deg is 111.32 in storm_watch and 110.574 elsewhere. | About 0.7% distance difference between the watch and the prefetch. Small but measurable. | **Pick one** (110.574 matches the WGS84 meridional value and the rest of the codebase). |
| D5 | `storms.detect_storms` empty return (6.10) | Empty result omits `threshold_dbz`, `cell_count`, `tracking_vectors`. | Consumers must treat keys as optional. | **Fix** by emitting the full shape. Keep `features: []`. |
| D6 | `storms._attach_tracking` (6.5) | `int(old_props.get("cell_id", next_id))` raises on a `None` stored value; `_parse`/subtraction raises `TypeError` for naive/aware mix. | Crash in ingest for a corrupted previous file. | **Fix** (validate; treat mixed tz as UTC). |
| D7 | `manifest.read_manifest_file` (3.4) | JSON that parses but lacks a `layers` dict returns an empty manifest. The next `update_manifest_file` writes that empty manifest and drops every layer. Parse errors raise (safe). | A structurally wrong but valid JSON file silently wipes history. | **Fix**: treat as error, like a parse failure. |
| D8 | `manifest` and `storms` and `nowcast_skill` writes (0.3) | No fsync before rename. | Power loss can leave a zero-length or stale file. Grids do fsync. | **Fix** (fsync tmp, then rename, then fsync dir) for `manifest.json` at minimum. |
| D9 | `state.ProcessedSet` (4.4, 4.5) | Non-list, non-dict JSON top level (number, null) raises from `__init__`. `max_entries=0` is unbounded. Duplicates from file not deduped on load. | Constructor crash on a bad state file blocks the activity. | **Fix** (treat as corrupt, reset). Make `max_entries=0` an error or explicit "unbounded". |
| D10 | `grid_dump.write_grid` (5.5 step 6) | For a non-NaN `fill`, NaN cells are stored as NaN (not replaced). | Readers may see NaN where other grids show `fill`. Callers (nowcast, storm grid) pass `-9999.0`, so it does not occur in practice. | Replicate; document. |
| D11 | `grid_dump.cleanup_old_grids` (5.7d) | Age sweep never removes `runs/*` directories. Incomplete generations (no marker) are never pruned at all. | Disk leak when nowcast stops finishing runs. | **Fix**: sweep run directories by age and by missing marker. |
| D12 | `grid_dump` age sweep (5.7d) | Quiet layers lose all their current `*.meta.json` and `*.bin` after 12 h without writes. | Intentional expiry, but the manifest may still list those timestamps. | Document; ensure tile cleanup prunes the manifest too. |
| D13 | `logger.JsonFormatter` (11.1) | `extra` keys `ts`, `level`, `service` overwrite the base fields. | Log lines can lie about level or service. | **Fix**: reserve those names (prefix extras). |
| D14 | `push_tokens` schema (9.2) | Docstring says unique per user + platform. Actual key is global `token`. Re-registration reassigns the token to a new user. | Possible cross-user push-token reassignment. | Decide policy. If intended, fix the docstring; if not, use a composite key. |
| D15 | `nowcast_skill.append_entries` (10.4) | Read-modify-write with no lock; corrupt log silently reset to empty. | Lost entries under concurrent writers; history loss on corruption. | Add the same flock as manifest; refuse to overwrite a corrupt file. |
| D16 | `nowcast_skill` timestamp policy (0.1) | Naive timestamps are UTC here but local-or-naive in storms and prefetch. | Subtle mis-sort if naive values appear. | Standardize on UTC-aware parsing everywhere. |
| D17 | `storm_watch.detect_change` (8.5) | `appeared` and `disappeared` kinds are declared and never produced. | Dead branches for clients. | Remove or implement. |
| D18 | `activity_heartbeat` (12.2) | Heartbeat exception masks the work result. | A transient heartbeat failure fails a successful activity. | **Fix**: catch and log heartbeat exceptions inside `_beat`. |
| D19 | Manifest lexicographic sort (3.9) | Sort is by string. Non-UTC or mixed `Z`/offset strings break chronology. The client requires chronological order. | Frames out of order in the app; validation rejects the whole manifest. | Normalize to UTC `+00:00` on write; reject others. |
| D20 | Mode of files (0.3) | `mkstemp` produces `0600`. The grid `.bin` is `0644`. | If the reader is another user, reads fail for meta but not bins (or vice versa). | Choose explicit modes. |
| D21 | `storms.write_storms_json` (6.9) | Equal timestamp is skipped (no rewrite) but the `radar` frame still publishes. | Benign, but the storm file is then stale relative to a re-render. | Document. |
| D22 | Nowcast frame `path` vs manifest v2 (3.3) | `path` is `runs/<anchor>/<valid>` and `timestamp` is the valid time. The `timestamps` list uses `timestamp`, so `frames[i].path` is not derivable from the timestamp. | Any consumer that builds tile URLs from `timestamp` will break (see D1). | Keep the rule: consumers MUST use `path`. |

---

## 15. Rust design notes

### 15.1 Crates (suggested, not mandatory)

| Need | Suggested crate | Notes |
|---|---|---|
| JSON | `serde`, `serde_json` | Use `BTreeMap` or a sorted serializer for writer style A. `serde_json::to_writer` with compact output matches separators `(",", ":")`. Writer style C (default separators, no newline) needs a custom formatter (`serde_json::ser::Formatter`) or hand-written output. Non-finite floats: `serde_json` errors. Python writes `NaN`. Decide: never write NaN (preferred), or emit the literal token. |
| Non-ASCII escaping | `serde_json` default is UTF-8 (no `\uXXXX`) | Python `ensure_ascii=True` escapes; the payload is still valid JSON and equal after parse. Byte compatibility requires a custom formatter. Recommended: compare semantically, not bytewise, except for grid binaries. |
| flock | `rustix::fs::flock` (LockExclusive/LockUnexclusive), or `fs4` | MUST be `flock(2)` (BSD), not `fcntl` record locks. |
| Temp file in same dir | `tempfile::NamedTempFile::new_in(dir)` then `persist(final)` | `NamedTempFile` creates `0600` (same as `mkstemp`). `persist` uses `rename(2)`. Add explicit `fsync` where section 0.3 idiom 2 applies. |
| SQLite | `rusqlite` with `bundled` feature | Set `busy_timeout(5s)`. Use `ON CONFLICT(token) DO UPDATE` (same SQL). Keep one connection per call or a pool, but match transaction boundaries. |
| Connected components | `ndarray` + hand-written two-pass union-find, 4-connectivity | MUST match scipy numbering (row-major first-pixel order) so tie order in the stable sort matches. Validate on golden fixtures. |
| Min-cost assignment | `pathfinding` (`kuhn_munkres`) with transposition for rectangular, or `lapjv` | Output must be an optimal assignment for rectangular `(n_cur, n_prev)` with entries `1e9` for missing predictions. Ties may differ from scipy. Use unique-cost fixtures. |
| Datetime | `chrono` (`DateTime<FixedOffset>`, `parse_from_rfc3339`) | Normalize `Z` to `+00:00`. Encode the naive-vs-UTC policy per module (0.1). |
| Logging | `tracing` + `tracing-subscriber` JSON layer, or a custom `Layer` | To match section 11 exactly, write a custom formatter that emits `ts` (`%Y-%m-%dT%H:%M:%S%z`), `level`, `service`, `msg`, `exc`, then extra fields, in that order. |
| Async, heartbeats | `tokio` + `tokio::task::spawn_blocking` + a select loop | Mirror section 12. Do not let heartbeat errors mask the result. |
| Numerics | `f64`, `f32` | Python `float` is `f64`; numpy grids are `f32`. Keep grid math in `f32` where Python does. |

### 15.2 Rounding and formatting (exact match required)

- **`round(x)` (no digits)**: Python uses round-half-to-even. Rust: `f64::round_ties_even` (stable since 1.77).
- **`round(x, n)` (decimal digits)**: Python rounds the exact binary value half-to-even. Rust: `format!("{:.n$}", x).parse::<f64>()` gives the same result. Do not use `(x*10^n).round()/10^n`.
- **`format(x, ".0f")`** (used in storm_watch summaries): Python rounds the exact binary value half-to-even. Rust `format!("{:.0}", x)` matches. Verify with `0.5 -> "0"`, `1.5 -> "2"`, `2.5 -> "2"`.
- **`str(float)`** (prometheus values): Python uses the shortest repr and always includes a decimal point (`1.0`). Rust `Display` prints `1`. Use `format!("{:?}", x)` for `1.0`, `0.833`, `0.0`. Values here are in `[0,1]` with three decimals, so exponent forms do not occur.
- **`math.radians`, `atan2`, `hypot`, `cos`, `asinh`**: match to ~1 ulp. Do not depend on bitwise equality; test with tolerances, except where a threshold comparison makes a 1-ulp difference visible (for example the 8 km gate). Use explicit `f64` throughout.
- **`isoformat()`** on aware UTC: `YYYY-MM-DDTHH:MM:SS[.ffffff]+00:00`. Microseconds appear only if non-zero. The Rust writer should emit `.ffffff` only when non-zero, to match.

### 15.3 Byte-exact grid writer

- Binary: `to_le_bytes` for each `f32`, row-major, no header. Test against a Python-produced golden file (see section 5.2 example).
- Meta: keys sorted (`data_file, fill, height, lat_max, lat_min, lon_max, lon_min, stride, unit, width`), compact, `"\n"` at end. `unit` escapes non-ASCII (`°F`).
- `uuid4().hex`: 32 lowercase hex chars, no hyphens.
- Lat orientation, lon normalization, stride, NaN rule: section 5.5.
- Lock order: layer lock, then tmp, then commit. Never take the layer lock while holding a file lock on a different path.

### 15.4 Test strategy

1. Golden fixtures produced by running the Python code (for example: `detect_storms` outputs on the `_grid` and `_cell_frame` fixtures, `write_grid` outputs, `summarize` outputs). Store under `testdata/` in the Rust tree.
2. Property tests for retention (keep-N, orphan grace, in-flight tmp).
3. Concurrency tests for lock behavior (blocking prune vs writer), using threads and `flock`.
4. Crash-injection tests (fail after bin write, after tmp write, after rename) for `write_grid`, manifest, ProcessedSet.
5. Contract tests: serialize manifest and validate against the rules in section 3.10 (port the TypeScript validator logic to Rust).

### 15.5 Migration and compatibility checklist

- [ ] Same file names and directory layout (section 2).
- [ ] `manifest.json`, `storms.json`, `nowcast-skill.json`, ProcessedSet files readable by both implementations during a rolling deploy.
- [ ] `flock` on the same sidecar files (`manifest.lock`, `storms.lock`, `.grid.lock`).
- [ ] Grid `.bin` and `.meta.json` names and contents identical.
- [ ] `runs/<run>/.grid-generation-complete-v1.json` identical, including `grid_metadata` sorting.
- [ ] Env var names and defaults (section 0.5).
- [ ] Decisions recorded for each item in section 14.

---

## Appendix A. Function index (public and private)

| Module | Function | Section |
|---|---|---|
| manifest | `empty_manifest`, `manifest_path`, `read_manifest_file`, `update_manifest_file`, `replace_layer_manifest` | 3.4 |
| manifest | `_normalize_frame`, `_sync_layer_indexes`, `_atomic_write_json` | 3.4, 3.5 |
| state | `ProcessedSet.__init__`, `__contains__`, `add`, `update`, `_load`, `_flush` | 4 |
| grid_dump | `write_grid`, `finalize_grid_generation`, `prune_grid_generations`, `prune_grid_layer`, `cleanup_old_grids` | 5.5 to 5.7 |
| grid_dump | `_prune_grid_layer_unlocked`, `_is_complete_grid_generation`, `_read_meta_data_file`, `_grid_timestamp_of`, `_relative_grid_key`, `_layer_lock`, `_durable_mkdir`, `_fsync_dir` | 5.7, 5.8 |
| storms | `detect_storms`, `write_storms_json` | 6.2 to 6.9 |
| storms | `_parse_timestamp`, `_haversine_km`, `_component_bbox`, `_translate_bbox`, `_previous_motion`, `_advance`, `_fit_motion`, `_attach_tracking` | 6.3 to 6.6 |
| storm_prefetch | `build_storm_prefetch_plan` | 7.2, 7.7 |
| storm_prefetch | `_lat_lon_to_tile`, `_tiles_for_bbox`, `_tile_path`, `_select_storm`, `_nearest_timestamp`, `_frame_for_lead`, `_distance_km` | 7.3 to 7.6 |
| storm_watch | `latest_radar_meta`, `sample_window`, `detect_change` | 8.3 to 8.5 |
| push_tokens | `upsert`, `delete_by_token`, `delete_for_user`, `list_for_user`, `_connect`, `_ensure_schema` | 9 |
| nowcast_skill | `skill_path`, `empty_log`, `read_log`, `append_entries`, `scores_from_counts`, `summarize`, `prometheus_lines`, `_ratio`, `_add_counts`, `_parse_ts` | 10 |
| logger | `JsonFormatter.format`, `get_logger`, `retry` | 11 |
| activity_heartbeat | `run_sync_with_heartbeat` | 12 |

## Appendix B. Verified caller contracts (for the port's integration tests)

- `ingest_mrms` radar frame commit (`ingest_mrms/activities.py` around 355 to 485): writes grid (`write_grid(layer, ts, ...)`, unit `dBZ`), writes `radar-nowcast-input` grid with `max_cells=NOWCAST_SCIENCE_GRID_MAX_CELLS` (default 7,000,000), prunes that layer, calls `kick_nowcast`, then `update_manifest_file(layer, ts, palettes=..., frame=..., layer_metadata={title, kind})`.
- `nowcast` run commit (`nowcast/activities.py` around 555 to 585): writes observed anchor grid at `runs/<anchor>/observed`, `finalize_grid_generation("nowcast", anchor)`, `replace_layer_manifest("nowcast", ...)`, `_prune_nowcast_point_grids(anchor)` (keep = `NOWCAST_POINT_GRID_RETENTION_RUNS`, default 36, minimum 2), then `ProcessedSet(STATE_DIR/"nowcast.json", max_entries=100).add(anchor)`.
- `nowcast` grid pointers: `write_grid("nowcast", valid, frame, lats, lons, "dBZ", -9999.0, POINT_GRID_MAX_CELLS, grid_key="runs/<anchor>/<valid>")`. `POINT_GRID_MAX_CELLS` default 900000. Frame values below 5 dBZ are set to `-9999.0`.
- `nowcast` manifest frames use `path = runs/<anchor>/<valid>`; `timestamp = valid` (see D1 and D22).
- `ingest_hrrr` and `ingest_airquality`: `replace_layer_manifest(layer, timestamps, palettes, state_dir, frames, layer_metadata={title, kind: "model_guidance", run_id, complete})`. Frame `path = runs/<run_id>/<valid>`.
- `tile_cleanup` and stale-dir sweeps: `update_manifest_file(layer, ts, action="remove")`.
- Storm watch: `compare_radar_frames` and `detect_storm_change` (section 8.6).
- Nowcast verification: `nowcast/skill.py::score_observation` writes entries (section 10.2) through `append_entries(state_dir, entries, now=now)`.
