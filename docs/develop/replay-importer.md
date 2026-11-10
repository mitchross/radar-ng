# Import historical MRMS tiles

`radar replay` imports the two cases in the [replay design](design/famous-storm-replay.md).
It is an operator CLI. It starts no API server or Temporal worker.
There are no web or app changes in this groundwork.

Check the anonymous NOAA archive without writing files:

```sh
radar replay check --case rolling-fork-2023
radar replay check --case greenfield-2024
radar replay import --case greenfield-2024 --revision pilot-1 --dry-run --max-frames 1
```

Run a small pilot on a scratch filesystem with at least 20 GiB free headroom:

```sh
mkdir -p /opt/data/cache/scratch/replay-pilot/tiles
radar replay import --case greenfield-2024 --revision pilot-1 \
  --tile-root /opt/data/cache/scratch/replay-pilot/tiles --max-frames 1 --verify-crop
```

`--max-frames` limits each selected layer. Repeat `--layer` to select layers.
Omitting both limits imports the whole case. Do not run a whole case before reviewing pilot measurements.
`--verify-crop` compares every selected PNG against an uncropped render, including boundary tiles.
It uses extra RAM, time and temporary quota space. It stops on any byte difference.
Only `classic` PNGs at zooms 4 through 7 are produced, with compression level 6. There are no packed buckets or retained point grids.

The archive inventory records exact keys, timestamps, nominal cadence gaps and unavailable playhead selections.
Counts describe listed objects. Each imported object must also download and decode successfully.
A failed download or decode leaves staging private and resumable. It never becomes an invented frame.
Object timestamps remain unchanged. Fast layers use the latest observation at or before the base playhead, within five minutes.
Hourly QPE keeps its own valid time and describes retrospective, rolling 24-hour precipitation.
Conversions and precipitation classification reuse live ingest helpers. Missing PrecipFlag stays explicit in provenance.

## Revision contract for the read-only API

Every write stays below `<tile-root>/replays/`. The importer never constructs a live ingest context.
It never writes the live manifest, point grids, nowcast inputs or processed-key database.
Live tile cleanup visits its fixed layer table and ignores `replays/`.
An exclusive replay import lock serializes all cases against the combined quota.

Imports first write `replays/<case>/.staging-<revision>/`. Repeat the exact command after an interruption.
The pinned plan rejects changed options, inventory or renderer sources. Completed frames need valid content-bound tiler markers.
An unfinished frame retries through the existing tiler publication protocol.
Corrupt or foreign output fails closed. Inspect it and choose a new revision; the importer never replaces it.
A finished revision appears through one durable directory rename to `replays/<case>/<revision>/`.
Published revision names are immutable. Existing revisions remain readable.

| File | Contract |
|---|---|
| `revision.json` | Schema 1 identity, case bounds/window, scope, source attribution, renderer fingerprint and readiness |
| `manifest.json` | Manifest v2 with case-only observations and immutable, namespaced PNG URLs |
| `coverage.json` | Full-window real object inventory and nominal gaps for six layers and PrecipFlag |
| `selection-gaps.json` | Base playheads with no eligible overlay observation; no interpolation |
| `plan.json` | Pinned selection, tile allowlist, archive URL, renderer source fingerprint and pilot limits |
| `progress.json` | Source and companion keys, SHA-256 checksums, completed frames and render timings |
| `import-report.json` | Run timing, allocated usage, inode counts, free bytes and crop comparisons |
| `tiles/<layer>/classic/<timestamp>/<z>/<x>/<y>.png` | Complete allowlisted tile footprints, including transparent PNGs |

`status: tiles_complete` means only that the advertised tile frames are verified.
Every revision and manifest carries `public_ready: false` and `warnings.status: missing` with issue 282.
A limited import has `scope: pilot`. It must never enter a public catalogue.
The read-only API in #283 must require full case coverage review and historical warnings from #282 before public publication.
Do not treat a directory, manifest or tile completion marker as public readiness.
Do not add a mutable latest alias. Pin warnings, manifest and tiles to one revision before advertising it.
Warnings integration must create a new immutable revision and verify its files before making that revision public.

## Quota and recovery

The 10 GiB quota includes allocated filesystem blocks for all revisions, staging, locks and parity output.
Before each frame, the importer reserves a conservative indexed-PNG and filesystem allocation bound.
It checks the quota and 20 GiB live free-space headroom before and after writes.
The reservation covers atomic frame publication and metadata replacement. Imports never evict live data.
The importer holds source GRIBs in memory for one frame, then drops them. It leaves no full-CONUS source files.
A pilot reports final allocated bytes after the revision rename; report-file measurements precede that file's own allocation.
The quota tracks blocks, not logical file lengths. Reservations account for the filesystem allocation unit.

Keep the exact binary and options when resuming. Renderer sources and Cargo.lock have a recorded SHA-256 fingerprint.
Interrupted staging remains counted against quota. A different revision preserves it for operator inspection.
To undo a code deployment, revert this change. Existing live data and publication paths remain independent.
No production cases are imported by this change.

Measured local pilot evidence is recorded in [replay-importer-pilot.md](replay-importer-pilot.md).
