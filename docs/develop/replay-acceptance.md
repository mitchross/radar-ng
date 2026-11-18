# Replay acceptance

Evidence for the import and web items of [#285](https://github.com/mitchross/radar-ng/issues/285).
The import follows the [publish runbook](replay-publish.md). The browser run uses the Playwright specs
in [`web/e2e`](../../web/e2e/replay.e2e.ts). Phone checks are tracked separately and are not covered here.

| Item | Local (2026-11-16) | Live |
|---|---|---|
| Both cases imported and published | Done, numbers below | Not yet: waits for the in-cluster import Job |
| Browser run: load, scrub, change layer, loop, slow network, missing data, return to live | 16 of 16 passed | Not yet: re-run the specs against the live site after the import |

## Local import

Source `feat/storm-sweep-finish` at d96e36f, release build, 6-core pod, 39 GiB RAM, Longhorn volume.
The runbook's code block was run as written, with four local substitutions: `TILE_DIR` and `W` point at scratch
directories, `df` and `curl` replace `busybox df` and `wget`. Exit status 0.

| | Greenfield 2024 | Rolling Fork 2023 |
|---|---|---|
| Import wall time | 28.2 min, dropped download, then 10.6 min resume | 30.9 min, dropped download, 4.7 min resume after one `retry` |
| Import peak RSS (first run) | 2.37 GiB | 2.38 GiB |
| Frames | 453 | 602 (+1 excluded object) |
| Tiles in the public revision | 7,701 | 9,632 |
| Boundary tiles verified on the last run | 2,924 | 2,000 |
| Warning history | 8 seed polygons, 123 versions, 124 bulletins, 134 IEM requests | 5 seed polygons, 110 versions, 110 bulletins, 117 IEM requests |
| Warnings sha256 | `f972ff93b6d60a5287b8638f704357eaebf5cec860a5e265e3f1b6352aba0737` | `c794dfbd948e0806918cc4691653bd77d6fe0d35c41a56a452974302cdaa2821` |
| Distinct archive object sha256 in `progress.json` | 543 | 721 |
| Public revision, apparent | 36,737,478 B | 45,584,225 B |
| Public revision, allocated | 79,171,584 B | 103,718,912 B |
| Files / inodes per public revision | 8,162 / 13,612 | 10,242 / 18,082 |

Whole tree after both publishes (private and public copies of both cases): 349 MB allocated, 157 MB apparent, 36,935 files.
Frame, tile, size and file counts match the 30-core measurements in the runbook exactly.

Per-layer coverage from the publish review (`frames` / coverage gaps / selection gaps):

| Layer | Greenfield | Rolling Fork |
|---|---|---|
| radar | 90 / 0 / 0 | 119 / 2 / 0 |
| radar-composite | 90 / 0 / 1 | 120 / 1 / 1 |
| hail-size | 90 / 1 / 0 | 120 / 1 / 0 |
| rotation-track | 90 / 1 / 0 | 120 / 1 / 0 |
| echo-top | 90 / 0 / 1 | 119 / 1 / 1 |
| precip-24h | 3 / 1 / 0 | 4 / 1 / 0 |

- Rolling Fork base gaps: 23:09:59 to 23:14:12 UTC (253 s, the excluded truncated object) and 00:00:10 to 00:04:07 UTC (237 s).
- Excluded: `MRMS_MergedBaseReflectivityQC_00.50_20230324-231159.grib2.gz`, 958,752 B, sha256 `5d8dfeca…ab9a71`, "GRIB2 parse error".
- Each case's composite and echo-top selection gap is the first playhead: their first object lands seconds after the first base scan.
- The other coverage gaps are the last nominal slot of the window.

After the run, the API served `/api/replays` with both cases. Each manifest and warnings file returned 200
and one tile per case returned `image/png`, both directly and through the web proxy.

Rollback was tested on Greenfield. `rm -rf` of `full-1-public` dropped the case from `/api/replays`, and its
manifest returned 404, within 2 s. Re-running the same publish took 52 s. It reported `created: true`, the same
warnings checksum and the same byte counts, and the case came back. Its `manifest.json` checksum differs
from the first publish, so compare live and local by the counts above, not by the manifest hash.

### What the run found

- **Dropped archive downloads.** The importer does not retry a connection that the archive closes mid-request
  ("connection closed before message completed"). This happened three times in five import runs, always about
  30 minutes in. Running the same command again resumed from the finished frames every time. The runbook now
  wraps each import in a three-try `retry`. The longer-term fix is a retry inside the importer.
- **Tools missing from the image.** The image has no `mkdir`, `df`, `rm` or `curl` on the `PATH`, but it ships
  the full `busybox` binary. The runbook now calls `busybox df` and `busybox rm` and uses `wget`, and writes the
  warnings to an `emptyDir`. Checked by `kubectl exec` (read-only) on the v2.0.43 tile-server pod.
- **Rollback cannot run from the tile-server pod**, which mounts the volume read-only. The runbook now says
  to run it as a one-off Job.

## Browser run

`bun run e2e` in `web/` (Playwright 1.65.1, Chrome Headless Shell 159, SwiftShader WebGL, 1280x800).
The local site was `vite preview` with `RADAR_REPLAYS=http://127.0.0.1:38291`: a local `radar api` over the
imported tree served the replay routes, and the live API served everything else.
Result: 16 passed, 0 failed, 5.1 min.

| Check, per case | Greenfield | Rolling Fork |
|---|---|---|
| Load: picker → first case tile | 712 ms | 1,030 ms |
| Scrub to the middle and the end, label and tile match the frame | pass, 90 frames | pass, 119 frames |
| Change layer: hail size, composite, past 24 h precip | pass (case tiles by layer: radar 164, hail 77, composite 66, precip 11) | pass (160, 50, 60, 10) |
| Loop at 4×, wraps from the end to the start | 1.8 s to wrap | 1.7 s to wrap |
| Slow network (400 ms RTT, 1.6 Mbit/s): first tile, scrub label, scrub tile | 2.1 s, 90 ms, 7.6 s | 2.0 s, 332 ms, 6.1 s |
| Missing data: recorded gaps | Echo tops at the first playhead says "No data for this layer at this time" | Same, and both base gaps are skipped, not invented |
| Missing data: one frame's tiles forced to 404 | 29 forced 404s, replay banner stays, later frame loads | 22 forced 404s, same |
| Missing data: manifest 503, then Retry | Error and Retry shown, no live radar, Retry loads the case | Same |
| Return to live | Banner gone, LIVE chip, live tiles load, no further case requests | Same |

Every check also asserts that no live `/tiles/` request is issued after a case is opened.

A tile that fails to load leaves the map empty for that frame with no notice. The timeline still shows the
historical time and the replay banner stays, so nothing live or invented is shown, but nothing says the frame is missing either.

Screenshots, traces and the JSON log stay outside the repo (`web/e2e/results/` and `web/e2e/report/` are git-ignored).

## Run it again

Local, against an imported tree:

```
cd web
TILE_DIR=<tree> LISTEN_ADDR=127.0.0.1:38291 radar api &        # from a release build of rust/
bunx vite build && RADAR_REPLAYS=http://127.0.0.1:38291 bunx vite preview --port 4173 &
PLAYWRIGHT_BROWSERS_PATH=<writable dir> bunx playwright install chromium
bun run e2e
```

Against a deployed site, after the import Job:

```
E2E_BASE_URL=https://radar.vanillax.me bun run e2e
```

`E2E_CASES=greenfield-2024` limits the run to one case. The browser run is not part of CI.
The web host must route `/replays` to the tile-server first; until then replay tiles come back as the HTML page.
