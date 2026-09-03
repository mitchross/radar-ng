# Replay fixtures

Layout: `<WorkflowType>/<version>/<scenario>.json` (`.json.gz` above 512 KiB).
`version` is the worker release whose workflow code produced the history
(`v1.1.26`), or `src-<sha>` for an unreleased checkout. A version directory is
never regenerated or edited: a new release gets a new directory and the old
ones stay, so `test_replay.py` proves the current code still replays every
retained release.

Every history is synthetic: real workflow code, fake activities, placeholder
inputs (`synthetic-*` identifiers, a `synthetic-placeholder` token, 0.0
coordinates, a unit-square polygon). None was exported from a production
namespace, and `test_replay.py` rejects anything that looks otherwise. Never
replace these files with production histories.

## Scenarios

| Workflow | Scenarios |
|---|---|
| IngestMrmsWorkflow | success, empty-backlog, partial (a frame reports unrendered), frame-activity-error (caught, run continues), list-activity-error (run fails) |
| IngestHrrrWorkflow | success, no-run, already-processed, partial (last hour pending), hour-activity-error (caught), publish-incoherent, find-activity-error |
| IngestAirQualityWorkflow | success, no-run, already-processed, partial (ozone unrendered), chunk-activity-error (PM2.5 lost, not marked), find-activity-error |
| PollAlertsWorkflow | geometry-fanout, zoneless-alerts, no-new-alerts, signal-activity-error (one alert isolated), fetch-activity-error |
| WatchStormWorkflow | signal-unpin, timer-poll, push-change, push-activity-error (watch survives), alert-signal-push, compare-activity-error, continue-as-new (1000 polls) |
| Lightning, Tropical, Nowcast, TileCleanup, RegisterPushToken, DeletePushToken, OpenMeteoSync | success, activity-error |

`REQUIRED_SCENARIOS` in `test_replay.py` is the subset every workflow must
keep, each with the event or activity that proves it took the named path.

## Adding a version

Generate from the released image so the fixtures record exactly the code that
shipped (the checked-in `v1.1.26` and `v1.1.27` sets were made this way):

```sh
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$PWD/temporal/workflows/replay_fixtures/generate.py:/gen/generate.py:ro" \
  -v "$PWD/temporal/workflows/replay_fixtures:/out" \
  --entrypoint python ghcr.io/mitchross/radar-ng-temporal-worker:vX.Y.Z \
  /gen/generate.py --version vX.Y.Z --out /out
```

From a source checkout (exact `temporalio` from `temporal/requirements.txt`;
the test-server binary downloads on first run):

```sh
python -m temporal.workflows.replay_fixtures.generate --version src-$(git rev-parse --short HEAD)
```

`generate.py` refuses to write into an existing version directory.
