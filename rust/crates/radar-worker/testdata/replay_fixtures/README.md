# Replay fixtures

Layout: `<WorkflowType>/<version>/<scenario>.json` (`.json.gz` above 512 KiB).
`version` is the worker release whose workflow code produced the history
(`v1.1.26`), or `src-<sha>` for an unreleased checkout. A version directory is
never regenerated or edited: the replay gate (`tests/replay.rs`) proves the
current workflows still replay every retained release.

All histories here were recorded from the Python worker releases (v1.1.26,
v1.1.27, v1.1.49, and `src-e8c8659`) before the Python code was removed.

Every history is synthetic: real workflow code, fake activities, placeholder
inputs (`synthetic-*` identifiers, a `synthetic-placeholder` token, 0.0
coordinates, a unit-square polygon). None was exported from a production
namespace. Never add production histories: real payloads carry user IDs, push
tokens, coordinates and alert geometry.

## Scenarios

| Workflow | Scenarios |
|---|---|
| IngestMrmsWorkflow | success, empty-backlog, partial (a frame reports unrendered), frame-activity-error (caught, run continues), list-activity-error (run fails) |
| IngestHrrrWorkflow | success, no-run, already-processed, partial (last hour pending), hour-activity-error (caught), publish-incoherent, find-activity-error |
| IngestAirQualityWorkflow | success, no-run, already-processed, partial (ozone unrendered), chunk-activity-error (PM2.5 lost, not marked), find-activity-error |
| PollAlertsWorkflow | geometry-fanout, zoneless-alerts, no-new-alerts, signal-activity-error (one alert isolated), fetch-activity-error |
| WatchStormWorkflow | signal-unpin, timer-poll, push-change, push-activity-error (watch survives), alert-signal-push, compare-activity-error, continue-as-new (1000 polls) |
| Lightning, Tropical, Nowcast, TileCleanup, RegisterPushToken, DeletePushToken, OpenMeteoSync | success, activity-error |

`REQUIRED_SCENARIOS` in `tests/replay.rs` is the subset every workflow must
keep, each with the event or activity that proves it took the named path.

## Generator

`generate.py` runs the Python workflow code against Temporal's time-skipping
test server, so it only works inside a Python worker image (`v1.1.x`), for
example:

```sh
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$PWD/rust/crates/radar-worker/testdata/replay_fixtures/generate.py:/gen/generate.py:ro" \
  -v "$PWD/rust/crates/radar-worker/testdata/replay_fixtures:/out" \
  --entrypoint python ghcr.io/mitchross/radar-ng-temporal-worker:v1.1.49 \
  /gen/generate.py --version v1.1.49 --out /out
```

There is no generator for histories recorded from the Rust workers yet.
`generate.py` refuses to write into an existing version directory.
