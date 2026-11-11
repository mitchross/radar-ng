# Publishing replay cases

`radar replay publish` turns a finished private import into a public revision the read-only API
([replay-api.md](replay-api.md)) lists. Part of #285.

## What it does

```
radar replay publish --case <id> --revision <private-rev> --warnings <file> --tile-root <dir>
                     [--published-revision <name>]    # default <private-rev>-public
```

1. Refuses anything that is not a finished whole-case import: scope must be `case` (not `pilot`),
   status `tiles_complete`, `public_ready` false, `plan.json` with no `--max-frames` or layer filter.
2. Reviews coverage and fails closed on the first mismatch: every planned frame has its full tile
   footprint (each tile a non-empty PNG, bound to its frame marker), `coverage.json` gaps and
   `selection-gaps.json` are recomputed from the object list and must match, every listed archive
   object was imported (except recorded exclusions), and `manifest.json` matches the verified frames.
3. Validates the warnings file against the case (`radar_alerts::replay::validate_file`, #282).
4. Builds a new revision in `.staging-<name>` (tiles copied, warnings added, `public_ready`,
   `coverage_reviewed` and `warnings.status: complete` set), verifies the copy, then renames it into place once.
5. The private revision is never touched. Re-running with the same inputs changes nothing and reports
   `"created": false`. Re-running with a different warnings file refuses: published revisions are immutable.

The copy doubles disk for a case. Cases are small (see below), so this is cheap.

## Undecodable archive objects

NOAA's archive can hold an object that downloads but does not decode. Rolling Fork has one:
`MergedBaseReflectivityQC_00.50_20230324-231159` is truncated (GRIB2 parse error). The importer
now supports leaving it out explicitly:

```
radar replay check --case <id> --decode      # downloads and decodes every object; lists "unusable"
radar replay import ... --exclude-object <key>   # repeat per key
```

An excluded key must download and fail to decode (a good object is refused). It is recorded in
`plan.json` and `coverage.json` (`excluded`, with reason and sha256), becomes an honest gap, and shows
in the publish review. `radar replay check` without `--decode` is unchanged.

## Measurements (full local import, optimized build, `--verify-crop`)

30-core host, 98 GiB RAM, local Longhorn volume. Both cases, whole window, no `--max-frames`.

| Step | Greenfield 2024 | Rolling Fork 2023 |
| --- | --- | --- |
| Import wall time | 19.8 min (1187 s) | 31.2 min (1873 s) |
| Import peak RSS | 2.42 GiB | 2.39 GiB |
| Frames | 453 | 602 (+1 excluded object) |
| Tiles verified at the crop boundary | 7,701 | 9,632 |
| Warning import | 7.4 s, 123 versions | 5.9 s, 110 versions |
| Publish wall time / RSS | 60.8 s / 0.02 GiB | 73.9 s / 0.02 GiB |
| Revision size, apparent | 36.7 MB | 45.6 MB |
| Revision size, allocated | 79.2 MB | 103.7 MB |
| Files / inodes per revision | 8,162 / 13,612 | 10,242 / 18,082 |

Whole tree after both publishes (private + public copies of both cases): 349 MB, 36,935 files.
`radar replay check --decode` for Rolling Fork took 17 min and 2.3 GiB peak.

Per-layer frames and gaps (from the publish review):

- Greenfield: radar 90, radar-composite 90 (1 selection gap), hail-size 90 (1 coverage gap),
  rotation-track 90 (1), echo-top 90 (1 selection gap), precip-24h 3 (1 coverage gap).
- Rolling Fork: radar 119 (2 coverage gaps: one missing, one excluded), radar-composite 120
  (1 coverage, 1 selection), hail-size 120 (1), rotation-track 120 (1), echo-top 119 (1 coverage, 1 selection), precip-24h 4.

Local check: `radar api` on that tree lists both cases at `/api/replays`; each manifest and
warnings file returns 200 and a sampled tile returns `image/png`.

## Operator runbook (in cluster)

Run in order, as one pod with the tile-server volume mounted at `/data/tiles` and outbound
internet. Use the `radar` image (`ghcr.io/mitchross/radar-ng-tile-server`; it contains the `radar` binary).
No writing to the volume while another replay job runs (the importer takes a lock and will refuse).

```
export TILE_DIR=/data/tiles
W=/data/replay-warnings            # any scratch dir on the volume
mkdir -p $W

# 0. Free space: each case is well under 1 GiB of output, but the importer
#    needs 20 GiB live free headroom (see replay-importer.md).
df -h /data/tiles

# 1. Greenfield
radar replay import --case greenfield-2024 --revision full-1 --verify-crop
radar import-replay-warnings greenfield-2024 $W/greenfield-2024.geojson
radar replay publish --case greenfield-2024 --revision full-1 --warnings $W/greenfield-2024.geojson

# 2. Rolling Fork (skip the one truncated object)
radar replay import --case rolling-fork-2023 --revision full-1 --verify-crop \
  --exclude-object CONUS/MergedBaseReflectivityQC_00.50/20230324/MRMS_MergedBaseReflectivityQC_00.50_20230324-231159.grib2.gz
radar import-replay-warnings rolling-fork-2023 $W/rolling-fork-2023.geojson
radar replay publish --case rolling-fork-2023 --revision full-1 --warnings $W/rolling-fork-2023.geojson

# 3. Check (the API caches readiness for 10 s)
curl -s http://<tile-server>/api/replays
```

Expect about 20 and 31 minutes for the two imports; publish about a minute each. Needs ~2.5 GiB
RAM. If an import fails midway, see replay-importer.md for resume and cleanup of `.staging-full-1`.
If the archive has since changed, `radar replay check --case <id> --decode` shows which objects
fail to decode before you start.

Every step is safe to re-run. If a publish says "immutable", the public revision already holds
different warnings; pick a new `--published-revision` (for example `full-1-public-2`).

### Rollback

Delete the public revision directory. The case leaves the catalogue within about 10 s; nothing else changes.

```
rm -rf /data/tiles/replays/<case>/full-1-public
```

The private `full-1` revision stays, so publishing again needs no re-import. To reclaim space,
delete `full-1` as well, but only after the public revision is gone or no longer needed.
