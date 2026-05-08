import asyncio
import time

from backend.shared import activity_heartbeat


def test_run_sync_with_heartbeat_beats_while_thread_runs(monkeypatch):
    heartbeats = []
    monkeypatch.setattr(activity_heartbeat.activity, "heartbeat", heartbeats.append)

    def slow_work():
        time.sleep(0.05)
        return "done"

    result = asyncio.run(
        activity_heartbeat.run_sync_with_heartbeat(
            slow_work,
            heartbeat_every=0.01,
            heartbeat_details={"phase": "test"},
        )
    )

    assert result == "done"
    assert heartbeats
    assert heartbeats[0] == {"phase": "test"}
