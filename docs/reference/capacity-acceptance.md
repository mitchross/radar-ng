# Capacity acceptance

The current target is 100 concurrently active map users. It is an acceptance
target, not a claim about measured production traffic. Run the k6 scenario
against a staging hostname backed by the same storage class and gateway:

```sh
k6 run -e BASE_URL=https://radar-staging.example load/k6-radar.js
```

The test ramps to 100 virtual users, holds for five minutes, and models one
manifest poll, a 3×3 radar tile viewport, and periodic point-forecast reads.
It passes only when server errors stay below 0.5%, total request failures below
1%, tile p95 below 800 ms, manifest p95 below 500 ms, and forecast p95 below
2 seconds.

Record CPU, memory, MRMS age, nowcast duration, Temporal queue backlog, PVC
latency, and gateway throughput during the run. A passing API test is invalid
if MRMS freshness breaches 10 minutes or the nowcast/HRRR schedules fall
behind. The system must serve users without starving publication.

The RWO tile PVC caps the current API at one pod. Static delivery
must move to immutable object storage/CDN before you call this multi-node or HA.
The single-pod test establishes headroom only for the present topology.

## Level II close-up

Level II has its own target from #293: 100 concurrent close-up users plus an overflow run and five
failure drills, against the shadow deployment. Script, Job and parameters:
[load/README.md](../../load/README.md#level-ii-close-up-k6-level2js). Drills:
[load/drills/README.md](../../load/drills/README.md).

A run passes only when all of these hold for its whole window:

- manifest p95 < 500 ms and cached tile p95 < 800 ms, measured at the edge (the shadow host), not
  only in-cluster;
- no ready manifest's newest frame is older than 10 minutes, and MRMS age stays at or below
  10 minutes;
- Temporal schedule-to-start on every radar-ng queue stays at or below one measured source
  cadence (the MRMS frame gap, or the Level II low-sweep gap if smaller);
- no promised tile is missing, server errors < 0.5 %, failed requests < 1 %, rate-limited < 1 %.

Cold views (submit to ready) are reported separately and are not counted as cached performance.
`load/drills/gate.py` scores the age, Temporal and k6 rules for a window. Record the results in the
[Level II launch-gate report](level2-launch-gate.md).

Level II API pods mount no RWO volume, so unlike the MRMS tile-server they can scale out. The
pool's limits (8 active stations, 2 decodes) cap Level II work regardless of users; demand past
them falls back to MRMS instead of queueing.
