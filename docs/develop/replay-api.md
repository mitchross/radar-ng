# Replay case API

Issue [#283](https://github.com/mitchross/radar-ng/issues/283) adds read-only routes for finished replay cases.
They serve files under `<TILE_DIR>/replays/<case>/<revision>/`. They write nothing, start no import and never fall back to live data.

| Route | Answer |
|---|---|
| `GET /api/replays` | Catalogue: one newest ready revision per case, with the exact URLs to use |
| `GET /api/replays/{id}/{revision}/manifest.json` | The revision's manifest v2, as stored |
| `GET /api/replays/{id}/{revision}/warnings.geojson` | The cached warning history, as stored (`application/geo+json`) |
| `GET /replays/{id}/{revision}/tiles/{layer}/classic/{frame}/{z}/{x}/{y}.png` | One immutable PNG |

`/api/manifest.json` is unchanged. `/tiles/replays/...` is a 404, so private files such as `revision.json` and `progress.json` stay private.

## When a revision is public

Everything must hold, or every route above answers 404 and the case drops out of the catalogue:

- `revision.json` has `public_ready: true`, `scope: "case"`, `coverage_reviewed: true`, `status: "tiles_complete"` and `warnings.status: "complete"`.
- Case id, revision, window, bounds and timezone match the importer's fixed case list.
- `manifest.json` is schema 2, names the same case and revision, and has the revision's own tile template.
- `warnings.geojson` is a schema 1 `FeatureCollection` for the same case and window.

The importer writes `public_ready: false`. Flipping it (and adding `warnings.geojson` and `coverage_reviewed`) is the publication step of ticket #285.

## Limits and caching

Case ids come from the importer's case list. Revisions use the importer's name rule. Frame, zoom and tile parts are checked before any file is opened, and every file must be a regular file under its cap.

| File | Cap |
|---|---|
| `revision.json` | 256 KiB |
| `manifest.json` | 8 MiB |
| `warnings.geojson` | 16 MiB |
| One tile | 1 MiB |

JSON answers carry an `ETag` and honour `If-None-Match` (`*`, weak and list forms too). Tiles are `public, max-age=31536000, immutable`; they are safe because a revision never changes.

## Shared contract fixture

[`web/src/shared/replayApi.fixture.json`](../../web/src/shared/replayApi.fixture.json) holds a sample catalogue, revision metadata and manifest.
The Rust test serves it. Web and the app parse it with `parseReplayCatalogue` in `web/src/shared/replayApi.ts`.
