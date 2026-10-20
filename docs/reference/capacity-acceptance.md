# Capacity acceptance

The current target is 100 concurrently active map users; it is an acceptance
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
behind—the system must serve users without starving publication.

The RWO tile PVC deliberately caps the current API at one pod. Static delivery
must move to immutable object storage/CDN before calling this multi-node or HA;
the single-pod test establishes headroom only for the present topology.
