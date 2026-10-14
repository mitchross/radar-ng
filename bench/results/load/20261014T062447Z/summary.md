# API load (bench 5)

Same fixture, same request list (local endpoints, tile URLs that exist), server pinned to one core, oha and the sampler to another. Latency in ms; CPU is % of the server's pinned core.

| target | run | achieved rps | success | p50 | p90 | p95 | p99 | server CPU % core | CPU ms/req | RSS peak MB | non-200 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| FastAPI + StaticFiles | 200 rps | 200.4 | 100.00% | 1.86 | 2.97 | 3.54 | 8.23 | 44.0 | 2.273 | 70.5 | none; 11 in flight at deadline |
| FastAPI + StaticFiles | 500 rps | 493.9 | 100.00% | 83.88 | 154.22 | 172.25 | 209.77 | 94.2 | 1.979 | 77.2 | none; 62 in flight at deadline |
| FastAPI + StaticFiles | 1000 rps | 571.7 | 100.00% | 106.65 | 146.72 | 160.79 | 193.8 | 95.6 | 1.734 | 78.0 | none; 64 in flight at deadline |
| FastAPI + StaticFiles | max (-c) | 538.0 | 100.00% | 111.88 | 160.06 | 181.91 | 214.13 | 94.9 | 1.832 | 78.5 | none; 64 in flight at deadline |
| radar-api | 200 rps | 200.5 | 100.00% | 0.37 | 0.52 | 0.63 | 1.9 | 4.6 | 0.238 | 11.7 | none; 15 in flight at deadline |
| radar-api | 500 rps | 500.0 | 100.00% | 0.38 | 0.59 | 0.76 | 2.23 | 11.1 | 0.229 | 12.1 | none; 2 in flight at deadline |
| radar-api | 1000 rps | 1000.0 | 100.00% | 0.31 | 0.49 | 0.61 | 2.55 | 16.7 | 0.173 | 12.5 | none |
| radar-api | max (-c) | 13039.4 | 100.00% | 4.05 | 5.83 | 6.72 | 45.81 | 94.9 | 0.075 | 14.1 | none; 64 in flight at deadline |
