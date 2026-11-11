# Level II tile tracer evidence (#289)

Acceptance mapping for slice B. Code is in [`radar-level2`](../../rust/crates/radar-level2/README.md). Design is in [level2-close-up.md](design/level2-close-up.md).

| Criterion | Evidence | Status |
|---|---|---|
| Real KGRR tile verified in RustFS | Render, immutable create and read-back are tested against an in-memory store. No real KGRR + RustFS run yet. | Open |
| Geometry ≤0.5 output pixel through z13 | `tests/tile_geometry.rs` against GeographicLib points in `testdata/tile-geometry.json` (near/far, altitude, bearings, wrap, negative tilt) | Pass |
| Seams, gaps, QC holes, range, transparent tiles | `tests/tile_sampler.rs` | Pass |
| Thin echoes vs denser reference, linear Z | `tests/tile_sampler.rs` | Pass |
| MRMS palette/class parity, all modes | `tests/tile_sampler.rs`: 354 value/palette/zoom checks | Pass |
| Real CPU, peak RSS, PNG bytes | Not measured yet | Open |
| 1 GiB / 512 MiB pool caps, stampede, oversized, cancel, timeout | `src/tile/tests.rs` with `LEVEL2_TEST_PG` on PostgreSQL 17 | Pass |
| fmt, clippy, workspace tests, docs links | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, CI `links` | Pass |

The two open rows need a real KGRR artifact and RustFS. Run the gated CLI from the crate README, then record the numbers here.
