"""radar-ng debug harness — inspect a live stack for performance and health issues.

Stdlib-only core (urllib) so it runs anywhere Python 3.10+ does: your laptop,
inside the tile-server container, or a k8s debug pod. The `temporal` command
additionally needs the `temporalio` package (already present in the worker
image; `pip install temporalio` elsewhere).

Usage:
    python -m tools.debug_harness doctor  --server http://host:8080
    python -m tools.debug_harness api      # endpoint latency percentiles
    python -m tools.debug_harness tiles    # tile fetch sampling + cache headers
    python -m tools.debug_harness pipeline # per-layer freshness vs cadence
    python -m tools.debug_harness client   # simulate the app's playback fetches
    python -m tools.debug_harness temporal # schedules + recent failures
    python -m tools.debug_harness watch    # doctor on a loop

Every command accepts --json for machine-readable output.
"""

__version__ = "0.1.0"
