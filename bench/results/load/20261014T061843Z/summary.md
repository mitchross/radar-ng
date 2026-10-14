# API load (bench 5)

Same fixture, same request list (local endpoints, tile URLs that exist), server pinned to one core, oha and the sampler to another. Latency in ms; CPU is % of the server's pinned core.

| target | run | achieved rps | success | p50 | p90 | p95 | p99 | server CPU % core | CPU ms/req | RSS peak MB | non-200 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| FastAPI + StaticFiles | 200 rps | 200.7 | 100.00% | 1.72 | 2.43 | 3.07 | 15.0 | 40.0 | 2.067 | 71.1 | none; 22 in flight at deadline |
| FastAPI + StaticFiles | 500 rps | 500.0 | 100.00% | 3.01 | 41.68 | 63.54 | 128.98 | 86.7 | 1.794 | 75.5 | none; 17 in flight at deadline |
| FastAPI + StaticFiles | 1000 rps | 362.8 | 100.00% | 173.66 | 228.82 | 243.07 | 297.76 | 95.2 | 2.73 | 77.1 | none; 64 in flight at deadline |
| FastAPI + StaticFiles | max (-c) | 453.8 | 100.00% | 136.78 | 180.11 | 199.97 | 239.14 | 94.9 | 2.174 | 78.0 | none; 64 in flight at deadline |
| radar-api | 200 rps | 200.1 | 100.00% | 0.41 | 0.64 | 0.77 | 1.98 | 5.4 | 0.277 | 11.4 | none; 2 in flight at deadline |
| radar-api | 500 rps | 500.0 | 100.00% | 0.42 | 0.67 | 0.89 | 2.67 | 12.2 | 0.251 | 11.6 | none; 1 in flight at deadline |
| radar-api | 1000 rps | 1000.1 | 100.00% | 0.36 | 0.63 | 0.82 | 2.7 | 19.0 | 0.196 | 11.9 | none; 6 in flight at deadline |
| radar-api | max (-c) | 9805.7 | 100.00% | 5.36 | 8.9 | 10.41 | 47.66 | 94.5 | 0.1 | 13.6 | none; 64 in flight at deadline |
