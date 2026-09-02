"""Generate sanitized synthetic replay histories for every radar-ng workflow.

Each scenario runs the real workflow code against Temporal's time-skipping test
server with fake activities, then exports the history to
``<out>/<WorkflowType>/<version>/<scenario>.json``. A version directory is
immutable once written: a new workflow release gets a new directory, and old
directories stay so the replay gate keeps proving compatibility with them.

From a source checkout (records the checked-out workflow code):

    python -m temporal.workflows.replay_fixtures.generate --version src-$(git rev-parse --short HEAD)

From a released worker image (records exactly the workflow code that shipped;
the script is self-contained so it also runs inside images that predate it):

    docker run --rm --user "$(id -u):$(id -g)" \\
      -v "$PWD/temporal/workflows/replay_fixtures/generate.py:/gen/generate.py:ro" \\
      -v "$PWD/temporal/workflows/replay_fixtures:/out" \\
      --entrypoint python ghcr.io/mitchross/radar-ng-temporal-worker:v1.1.26 \\
      /gen/generate.py --version v1.1.26 --out /out

Never export production histories: real payloads carry user IDs, push tokens,
coordinates, alert geometry, and failure details. Every input here is a
placeholder and the sanitizer refuses to write anything else.
"""

from __future__ import annotations

import argparse
import asyncio
import functools
import gzip
import json
import logging
import sys
from collections import Counter
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from temporalio import activity
from temporalio.client import (
    Client,
    WorkflowContinuedAsNewError,
    WorkflowFailureError,
    WorkflowHandle,
)
from temporalio.exceptions import ApplicationError
from temporalio.testing import WorkflowEnvironment
from temporalio.worker import Worker

from backend.api.api import storm_watch_activities as storm
from backend.ingest_airquality import activities as aqm
from backend.ingest_hrrr import activities as hrrr
from backend.ingest_lightning import activities as lightning
from backend.ingest_mrms import activities as mrms
from backend.ingest_tropical import activities as tropical
from backend.nowcast import activities as nowcast
from backend.open_meteo_sync import activities as open_meteo
from backend.tile_cleanup import activities as tile_cleanup
from temporal.task_queues import OPEN_METEO_TASK_QUEUE
from temporal.workflows.ingest_airquality import IngestAirQualityWorkflow
from temporal.workflows.ingest_hrrr import IngestHrrrWorkflow
from temporal.workflows.ingest_lightning import IngestLightningWorkflow
from temporal.workflows.ingest_mrms import IngestMrmsWorkflow
from temporal.workflows.ingest_tropical import IngestTropicalWorkflow
from temporal.workflows.nowcast import NowcastWorkflow
from temporal.workflows.open_meteo_sync import OpenMeteoSyncWorkflow
from temporal.workflows.poll_alerts import PollAlertsWorkflow
from temporal.workflows.register_push_token import (
    DeletePushTokenWorkflow,
    RegisterPushTokenInput,
    RegisterPushTokenWorkflow,
)
from temporal.workflows.tile_cleanup import TileCleanupWorkflow
from temporal.workflows.watch_storm import WatchStormInput, WatchStormWorkflow

DEFAULT_OUT = Path(__file__).parent
TASK_QUEUE = "synthetic-replay-fixtures"
CLIENT_IDENTITY = "synthetic-replay-client"
WORKER_IDENTITY = "synthetic-replay-worker"
# Histories above this size (the continue-as-new run) are stored gzip-compressed.
GZIP_THRESHOLD_BYTES = 512 * 1024
SYNTHETIC_RUN = "synthetic-run"
# Unit square at null island; the replay test only accepts 0/1 coordinates.
SYNTHETIC_GEOMETRY = {
    "type": "Polygon",
    "coordinates": [[[0, 0], [0, 1], [1, 1], [0, 0]]],
}
NO_ARG = object()

Behavior = Callable[["Recorder", int, tuple[Any, ...]], Any]


class Recorder:
    """Counts fake-activity calls and lets a scenario driver hold/release one."""

    def __init__(self) -> None:
        self.calls: Counter[str] = Counter()
        self._cond = asyncio.Condition()
        self._releases: dict[tuple[str, int], asyncio.Event] = {}

    async def record(self, name: str) -> int:
        async with self._cond:
            self.calls[name] += 1
            self._cond.notify_all()
            return self.calls[name]

    async def wait_for(self, name: str, count: int) -> None:
        async with self._cond:
            await self._cond.wait_for(lambda: self.calls[name] >= count)

    def release(self, name: str, call: int) -> None:
        self._releases.setdefault((name, call), asyncio.Event()).set()

    async def held(self, name: str, call: int) -> None:
        await self._releases.setdefault((name, call), asyncio.Event()).wait()


@dataclass
class Scenario:
    workflow: type
    name: str
    arg: Any = NO_ARG
    behaviors: dict[str, Behavior] = field(default_factory=dict)
    driver: Callable[[WorkflowHandle, Recorder], Awaitable[None]] | None = None
    outcome: str = "completed"  # completed | failed | continued_as_new


_current: Scenario | None = None
_recorder: Recorder | None = None


def ret(value: Any) -> Behavior:
    return lambda _rec, _call, _args: value


def fail(message: str) -> Behavior:
    def behavior(_rec: Recorder, _call: int, _args: tuple[Any, ...]) -> Any:
        raise ApplicationError(message, type="synthetic-failure", non_retryable=True)

    return behavior


def by_args(fn: Callable[..., Any]) -> Behavior:
    return lambda _rec, _call, args: fn(*args)


async def _dispatch(name: str, args: tuple[Any, ...]) -> Any:
    assert _current is not None and _recorder is not None
    call = await _recorder.record(name)
    behavior = _current.behaviors.get(name) or DEFAULTS[name]
    result = behavior(_recorder, call, args)
    if asyncio.iscoroutine(result):
        result = await result
    return result


def _fake(real: Callable[..., Any]) -> Callable[..., Any]:
    name = activity._Definition.must_from_callable(real).name
    assert name is not None

    @functools.wraps(
        real, updated=()
    )  # real signature, so payloads decode to dataclasses
    async def impl(*args: Any) -> Any:
        return await _dispatch(name, args)

    return activity.defn(name=name)(impl)


REAL_ACTIVITIES = [
    mrms.mrms_list_unprocessed_keys,
    mrms.mrms_process_frame,
    mrms.mrms_mark_processed,
    mrms.mrms_cleanup,
    hrrr.hrrr_find_latest_run,
    hrrr.hrrr_horizon_for_run,
    hrrr.hrrr_process_forecast_hour,
    hrrr.hrrr_mark_processed,
    hrrr.hrrr_publish_run,
    hrrr.hrrr_cleanup,
    aqm.aqm_find_latest_run,
    aqm.aqm_render_chunk,
    aqm.aqm_publish_run,
    aqm.aqm_mark_processed,
    aqm.aqm_cleanup,
    lightning.lightning_consume_stream,
    tropical.tropical_fetch_and_publish,
    nowcast.nowcast_run,
    tile_cleanup.tile_cleanup_sweep,
    storm.fetch_nws_active_alerts,
    storm.signal_matching_storm_watches,
    storm.mark_alerts_seen,
    storm.compare_radar_frames,
    storm.detect_storm_change,
    storm.fan_out_push_to_user,
    storm.persist_push_token,
    storm.delete_push_token,
]
FAKE_ACTIVITIES = [_fake(real) for real in REAL_ACTIVITIES]
FAKE_OPEN_METEO = _fake(open_meteo.open_meteo_sync)

MRMS_KEYS = ["synthetic-key-1", "synthetic-key-2", "synthetic-key-3"]
HRRR_HORIZON = 3
HRRR_LAYERS = ["radar-hrrr", "synthetic-layer"]
AQM_TIMESTAMPS = [f"synthetic-ts-{i}" for i in range(aqm.CHUNK_MESSAGES)]
ALERT_IDS = ["synthetic-alert-1", "synthetic-alert-2", "synthetic-alert-3"]


def _frame(
    inp: mrms.ProcessFrameInput, rendered: bool = True
) -> mrms.ProcessFrameResult:
    return mrms.ProcessFrameResult(
        key=inp.key, timestamp="synthetic-ts", rendered=rendered, palettes=["synthetic"]
    )


def _hour(
    _run_id: str, fhr: int, layers: list[str] | None = None
) -> hrrr.ForecastHourResult:
    return hrrr.ForecastHourResult(
        fhr=fhr,
        rendered_layers=HRRR_LAYERS if layers is None else layers,
        duration_s=0.0,
    )


def _chunk(
    _run_id: str, layer: str, start_msg: int, rendered: bool = True
) -> aqm.AqmChunkResult:
    return aqm.AqmChunkResult(
        layer=layer,
        start_msg=start_msg,
        rendered_timestamps=AQM_TIMESTAMPS if rendered else [],
    )


def _alerts(with_geometry: bool) -> storm.FetchAlertsResult:
    geometry = SYNTHETIC_GEOMETRY if with_geometry else {}
    return storm.FetchAlertsResult(
        alert_count=5,
        new_alert_ids=list(ALERT_IDS),
        new_alerts=[
            storm.AlertForSignal(alert_id=aid, geometry=dict(geometry))
            for aid in ALERT_IDS
        ],
    )


def _compare(sampled: bool) -> storm.CompareFramesResult:
    if not sampled:
        return storm.CompareFramesResult(sampled=False)
    return storm.CompareFramesResult(
        sampled=True, curr_timestamp="synthetic-ts", curr_max_dbz=55.0, has_prev=False
    )


DEFAULTS: dict[str, Behavior] = {
    "mrms_list_unprocessed_keys": ret(
        mrms.ListKeysResult(keys=list(MRMS_KEYS), backlog_total=3)
    ),
    "mrms_process_frame": by_args(_frame),
    "mrms_mark_processed": ret(None),
    "mrms_cleanup": ret(mrms.CleanupResult(tile_dirs_removed=1, grid_files_removed=1)),
    "hrrr_find_latest_run": ret(
        hrrr.FindRunResult(run_id=SYNTHETIC_RUN, already_processed=False)
    ),
    "hrrr_horizon_for_run": ret(HRRR_HORIZON),
    "hrrr_process_forecast_hour": by_args(_hour),
    "hrrr_publish_run": ret(list(HRRR_LAYERS)),
    "hrrr_mark_processed": ret(None),
    "hrrr_cleanup": ret(
        hrrr.HrrrCleanupResult(tile_dirs_removed=1, grid_files_removed=1)
    ),
    "aqm_find_latest_run": ret(
        aqm.AqmFindRunResult(run_id=SYNTHETIC_RUN, already_processed=False)
    ),
    "aqm_render_chunk": by_args(_chunk),
    "aqm_publish_run": ret(list(aqm.AQM_LAYERS)),
    "aqm_mark_processed": ret(None),
    "aqm_cleanup": ret(aqm.AqmCleanupResult(tile_dirs_removed=1)),
    "lightning_consume_stream": ret(
        lightning.LightningRunResult(
            duration_s=1.0, msgs=3, parsed=3, in_bbox=2, final_buffer=2
        )
    ),
    "tropical_fetch_and_publish": ret(
        tropical.TropicalResult(storm_count=1, feature_count=2)
    ),
    "nowcast_run": ret(nowcast.NowcastResult(ran=True)),
    "tile_cleanup_sweep": ret(
        tile_cleanup.TileCleanupResult(
            layers_swept=2, tile_dirs_removed=1, grid_files_removed=1
        )
    ),
    "fetch_nws_active_alerts": ret(_alerts(with_geometry=True)),
    "signal_matching_storm_watches": ret(storm.SignalWatchesResult(matched=1)),
    "mark_alerts_seen": ret(None),
    "compare_radar_frames": ret(_compare(sampled=False)),
    "detect_storm_change": ret(
        storm.DetectChangeResult(kind="intensifying", summary="synthetic")
    ),
    "fan_out_push_to_user": ret(storm.FanOutPushResult(sent=2)),
    "persist_push_token": ret(None),
    "delete_push_token": ret(1),
    "open_meteo_sync": by_args(
        lambda args: open_meteo.OpenMeteoSyncResult(
            model=args.model, succeeded=True, duration_s=1.0
        )
    ),
}

FAILURE = fail("synthetic-activity-failure")


def _fail_now() -> Any:
    return FAILURE(Recorder(), 0, ())


MRMS_ARGS = mrms.IngestMrmsArgs(
    mrms_prefix="synthetic-prefix", layer_name="synthetic-layer"
)
WATCH_INPUT = WatchStormInput(
    user_id="synthetic-user", storm_cell_id="synthetic-storm", lat=0.0, lng=0.0
)
PUSH_INPUT = RegisterPushTokenInput(
    user_id="synthetic-user",
    token="synthetic-placeholder",
    platform="synthetic-platform",
)
OPEN_METEO_ARGS = open_meteo.OpenMeteoSyncArgs(
    model="synthetic-model", variables="synthetic-variable", past_days=0
)


def _unpin_after(
    hold_call: int,
) -> Callable[[WorkflowHandle, Recorder], Awaitable[None]]:
    async def driver(handle: WorkflowHandle, rec: Recorder) -> None:
        await rec.wait_for("compare_radar_frames", hold_call)
        await handle.signal(WatchStormWorkflow.unpin)
        rec.release("compare_radar_frames", hold_call)

    return driver


async def _alert_then_unpin(handle: WorkflowHandle, rec: Recorder) -> None:
    await rec.wait_for("compare_radar_frames", 1)
    await handle.signal(WatchStormWorkflow.alert_match, "synthetic-alert-1")
    rec.release("compare_radar_frames", 1)
    await rec.wait_for("compare_radar_frames", 2)
    await handle.signal(WatchStormWorkflow.unpin)
    rec.release("compare_radar_frames", 2)


def _hold_compare(*calls: int) -> Behavior:
    async def behavior(rec: Recorder, call: int, _args: tuple[Any, ...]) -> Any:
        if call in calls:
            await rec.held("compare_radar_frames", call)
        return _compare(sampled=False)

    return behavior


async def _compare_hold_second(rec: Recorder, call: int, _args: tuple[Any, ...]) -> Any:
    # First poll samples a frame (push path); the second is held until unpin.
    if call == 1:
        return _compare(sampled=True)
    await rec.held("compare_radar_frames", call)
    return _compare(sampled=False)


def _single(workflow: type, name: str, arg: Any = NO_ARG) -> list[Scenario]:
    return [
        Scenario(workflow, "success", arg),
        Scenario(workflow, "activity-error", arg, {name: FAILURE}, outcome="failed"),
    ]


def _mrms_frame_error(inp: mrms.ProcessFrameInput) -> mrms.ProcessFrameResult:
    return _frame(inp) if inp.key != MRMS_KEYS[1] else _fail_now()


def _hrrr_hour_error(run_id: str, fhr: int) -> hrrr.ForecastHourResult:
    return _hour(run_id, fhr) if fhr != 2 else _fail_now()


def _hrrr_hour_pending(run_id: str, fhr: int) -> hrrr.ForecastHourResult:
    return _hour(run_id, fhr, [] if fhr == HRRR_HORIZON else None)


def _aqm_chunk_error(run_id: str, layer: str, start: int) -> aqm.AqmChunkResult:
    return _chunk(run_id, layer, start) if layer != "air-quality" else _fail_now()


def _aqm_chunk_partial(run_id: str, layer: str, start: int) -> aqm.AqmChunkResult:
    return _chunk(run_id, layer, start, rendered=layer == "air-quality")


def _signal_error(inp: storm.SignalWatchesInput) -> storm.SignalWatchesResult:
    return (
        storm.SignalWatchesResult(matched=1)
        if inp.alert_id != ALERT_IDS[1]
        else _fail_now()
    )


SCENARIOS: list[Scenario] = [
    Scenario(IngestMrmsWorkflow, "success", MRMS_ARGS),
    Scenario(
        IngestMrmsWorkflow,
        "empty-backlog",
        behaviors={
            "mrms_list_unprocessed_keys": ret(
                mrms.ListKeysResult(keys=[], backlog_total=0)
            ),
        },
    ),
    Scenario(
        IngestMrmsWorkflow,
        "partial",
        MRMS_ARGS,
        {
            "mrms_process_frame": by_args(
                lambda inp: _frame(inp, rendered=inp.key != MRMS_KEYS[1])
            )
        },
    ),
    Scenario(
        IngestMrmsWorkflow,
        "frame-activity-error",
        MRMS_ARGS,
        {"mrms_process_frame": by_args(_mrms_frame_error)},
    ),
    Scenario(
        IngestMrmsWorkflow,
        "list-activity-error",
        MRMS_ARGS,
        {"mrms_list_unprocessed_keys": FAILURE},
        outcome="failed",
    ),
    Scenario(IngestHrrrWorkflow, "success"),
    Scenario(
        IngestHrrrWorkflow,
        "no-run",
        behaviors={
            "hrrr_find_latest_run": ret(
                hrrr.FindRunResult(run_id=None, already_processed=False)
            ),
        },
    ),
    Scenario(
        IngestHrrrWorkflow,
        "already-processed",
        behaviors={
            "hrrr_find_latest_run": ret(
                hrrr.FindRunResult(run_id=SYNTHETIC_RUN, already_processed=True)
            ),
        },
    ),
    Scenario(
        IngestHrrrWorkflow,
        "partial",
        behaviors={"hrrr_process_forecast_hour": by_args(_hrrr_hour_pending)},
    ),
    Scenario(
        IngestHrrrWorkflow,
        "hour-activity-error",
        behaviors={"hrrr_process_forecast_hour": by_args(_hrrr_hour_error)},
    ),
    Scenario(
        IngestHrrrWorkflow,
        "publish-incoherent",
        behaviors={"hrrr_publish_run": ret([])},
    ),
    Scenario(
        IngestHrrrWorkflow,
        "find-activity-error",
        behaviors={"hrrr_find_latest_run": FAILURE},
        outcome="failed",
    ),
    Scenario(IngestAirQualityWorkflow, "success"),
    Scenario(
        IngestAirQualityWorkflow,
        "no-run",
        behaviors={
            "aqm_find_latest_run": ret(
                aqm.AqmFindRunResult(run_id=None, already_processed=False)
            ),
        },
    ),
    Scenario(
        IngestAirQualityWorkflow,
        "already-processed",
        behaviors={
            "aqm_find_latest_run": ret(
                aqm.AqmFindRunResult(run_id=SYNTHETIC_RUN, already_processed=True)
            ),
        },
    ),
    Scenario(
        IngestAirQualityWorkflow,
        "partial",
        behaviors={
            "aqm_render_chunk": by_args(_aqm_chunk_partial),
            "aqm_publish_run": ret(["air-quality"]),
        },
    ),
    Scenario(
        IngestAirQualityWorkflow,
        "chunk-activity-error",
        behaviors={
            "aqm_render_chunk": by_args(_aqm_chunk_error),
            "aqm_publish_run": ret(["ozone"]),
        },
    ),
    Scenario(
        IngestAirQualityWorkflow,
        "find-activity-error",
        behaviors={"aqm_find_latest_run": FAILURE},
        outcome="failed",
    ),
    Scenario(PollAlertsWorkflow, "geometry-fanout"),
    Scenario(
        PollAlertsWorkflow,
        "zoneless-alerts",
        behaviors={"fetch_nws_active_alerts": ret(_alerts(with_geometry=False))},
    ),
    Scenario(
        PollAlertsWorkflow,
        "no-new-alerts",
        behaviors={
            "fetch_nws_active_alerts": ret(storm.FetchAlertsResult(alert_count=5))
        },
    ),
    Scenario(
        PollAlertsWorkflow,
        "signal-activity-error",
        behaviors={"signal_matching_storm_watches": by_args(_signal_error)},
    ),
    Scenario(
        PollAlertsWorkflow,
        "fetch-activity-error",
        behaviors={"fetch_nws_active_alerts": FAILURE},
        outcome="failed",
    ),
    Scenario(
        WatchStormWorkflow,
        "signal-unpin",
        WATCH_INPUT,
        {"compare_radar_frames": _hold_compare(1)},
        driver=_unpin_after(1),
    ),
    Scenario(
        WatchStormWorkflow,
        "timer-poll",
        WATCH_INPUT,
        {"compare_radar_frames": _hold_compare(2)},
        driver=_unpin_after(2),
    ),
    Scenario(
        WatchStormWorkflow,
        "push-change",
        WATCH_INPUT,
        {"compare_radar_frames": _compare_hold_second},
        driver=_unpin_after(2),
    ),
    Scenario(
        WatchStormWorkflow,
        "push-activity-error",
        WATCH_INPUT,
        {"compare_radar_frames": _compare_hold_second, "fan_out_push_to_user": FAILURE},
        driver=_unpin_after(2),
    ),
    Scenario(
        WatchStormWorkflow,
        "alert-signal-push",
        WATCH_INPUT,
        {"compare_radar_frames": _hold_compare(1, 2)},
        driver=_alert_then_unpin,
    ),
    Scenario(
        WatchStormWorkflow,
        "compare-activity-error",
        WATCH_INPUT,
        {"compare_radar_frames": FAILURE},
        outcome="failed",
    ),
    Scenario(
        WatchStormWorkflow, "continue-as-new", WATCH_INPUT, outcome="continued_as_new"
    ),
    *_single(IngestLightningWorkflow, "lightning_consume_stream"),
    *_single(IngestTropicalWorkflow, "tropical_fetch_and_publish"),
    *_single(NowcastWorkflow, "nowcast_run"),
    *_single(TileCleanupWorkflow, "tile_cleanup_sweep"),
    *_single(RegisterPushTokenWorkflow, "persist_push_token", PUSH_INPUT),
    *_single(DeletePushTokenWorkflow, "delete_push_token", "synthetic-placeholder"),
    *_single(OpenMeteoSyncWorkflow, "open_meteo_sync", OPEN_METEO_ARGS),
]

WORKFLOWS = sorted({s.workflow for s in SCENARIOS}, key=lambda w: w.__name__)


def _sanitize(document: Any) -> Any:
    if isinstance(document, dict):
        clean: dict[str, Any] = {}
        for key, value in document.items():
            if key == "stackTrace":
                continue  # generator file paths, not workflow behaviour
            if key == "identity" and not str(value).startswith("synthetic-"):
                raise ValueError(f"non-synthetic identity in history: {value!r}")
            clean[key] = _sanitize(value)
        return clean
    if isinstance(document, list):
        return [_sanitize(item) for item in document]
    return document


def _write(path: Path, history_json: str) -> Path:
    body = json.dumps(_sanitize(json.loads(history_json)), indent=2) + "\n"
    data = body.encode()
    if len(data) > GZIP_THRESHOLD_BYTES:
        path = path.with_suffix(".json.gz")
        data = gzip.compress(data, mtime=0)
    # Release fixture versions are append-only. Exclusive creation makes the
    # immutability rule hold even if two generator processes race.
    with path.open("xb") as output:
        output.write(data)
    return path


async def _run_scenario(client: Client, scenario: Scenario, version: str) -> str:
    global _current, _recorder
    _current, _recorder = scenario, Recorder()

    workflow_id = f"synthetic-{version}-{scenario.workflow.__name__}-{scenario.name}"
    args = [] if scenario.arg is NO_ARG else [scenario.arg]
    handle = await client.start_workflow(
        scenario.workflow.run, *args, id=workflow_id, task_queue=TASK_QUEUE
    )
    driver = (
        asyncio.create_task(scenario.driver(handle, _recorder))
        if scenario.driver
        else None
    )
    outcome = "completed"
    try:
        await handle.result(follow_runs=False)
    except WorkflowContinuedAsNewError:
        outcome = "continued_as_new"
        await client.get_workflow_handle(workflow_id).terminate(
            "synthetic fixture captured"
        )
    except WorkflowFailureError:
        outcome = "failed"
    if driver:
        await driver
    if outcome != scenario.outcome:
        raise RuntimeError(f"{workflow_id}: expected {scenario.outcome}, got {outcome}")

    first_run = client.get_workflow_handle(
        workflow_id, run_id=handle.first_execution_run_id
    )
    return (await first_run.fetch_history()).to_json()


async def generate(out: Path, version: str, only: set[str] | None) -> None:
    targets = [s for s in SCENARIOS if only is None or s.workflow.__name__ in only]
    for workflow in {s.workflow for s in targets}:
        version_dir = out / workflow.__name__ / version
        if version_dir.exists() and any(version_dir.iterdir()):
            raise SystemExit(
                f"{version_dir} already exists; fixture versions are immutable"
            )

    async with await WorkflowEnvironment.start_time_skipping(
        identity=CLIENT_IDENTITY
    ) as env:
        async with (
            Worker(
                env.client,
                task_queue=TASK_QUEUE,
                workflows=WORKFLOWS,
                activities=FAKE_ACTIVITIES,
                identity=WORKER_IDENTITY,
            ),
            # OpenMeteoSyncWorkflow dispatches its activity to the Swift worker's queue.
            Worker(
                env.client,
                task_queue=OPEN_METEO_TASK_QUEUE,
                activities=[FAKE_OPEN_METEO],
                identity=f"{WORKER_IDENTITY}-open-meteo",
            ),
        ):
            for scenario in targets:
                history_json = await _run_scenario(env.client, scenario, version)
                version_dir = out / scenario.workflow.__name__ / version
                version_dir.mkdir(parents=True, exist_ok=True)
                written = _write(version_dir / f"{scenario.name}.json", history_json)
                print(f"wrote {written.relative_to(out)}")


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True, help="worker release, e.g. v1.1.26")
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument(
        "--workflow", action="append", help="limit to these workflow type names"
    )
    opts = parser.parse_args(argv)
    only = set(opts.workflow) if opts.workflow else None
    logging.getLogger("temporalio").setLevel(
        logging.ERROR
    )  # expected fake failures are noisy
    asyncio.run(generate(opts.out, opts.version, only))


if __name__ == "__main__":
    main(sys.argv[1:])
