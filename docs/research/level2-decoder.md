# Archive II decoder decision

Research for [Which Rust decoder for Archive II Message 31?](https://github.com/mitchross/radar-ng/issues/274). Checked 2026-11-08 EST / 2026-11-09 UTC. No product changes.

## Decision

Use **danielway/nexrad 1.0.0** behind a radar-ng-owned adapter. Do not write a new Archive II/Message 31 parser. Start with:

```toml
nexrad-data = { version = "=1.0.0", default-features = false, features = ["model"] }
```

Keep networking, retries, resource limits, sweep selection and publication in our server. Add a direct `nexrad-decode` dependency when low-level selective conversion is needed. Disable default AWS features to reuse our server-side download policy. Leave optional `parallel` off until worker-level concurrency is measured.

The published crate decoded a real KGRR volume on the project's Rust 1.97.0 toolchain. REF, ZDR, RHO (CC) and PHI were all present and iterated successfully. This establishes feasibility, not scientific correctness or production capacity.

## Comparison

| Choice | Maintenance / licence | Format and compression | Verdict |
| --- | --- | --- | --- |
| `nexrad-data`, `nexrad-decode`, `nexrad-model` 1.0.0 | MIT; first stable release 2026-10-27; MSRV 1.88.0. Inspected main `ae8b842ddfcb61fe7174db0e33789bcb5e3f03d2`. GitHub reported 27 stars, zero open issues and not archived. These are activity signals, not reliability guarantees. | Archive II framing, bzip2 LDM records, older gzip files; modern Message 31 and legacy Message 1; explicit REF/ZDR/RHO/PHI, 8/16-bit gates, scale/offset. | Adopt; isolate types and pin initially. |
| `rustdar-core` in FahrenheitResearch/rustdar | MIT, path library 0.1.0 in a viewer. Latest inspected commit `3a708877b3f7bb5a93e0d14a95df36a3e0ada698`, 2026-04-04. | Dual-pol names and bzip2. Source falls back to input bytes on decompression failure and can return partial data on parse failure; raw 0/1 both become NaN. | Less suitable error/data semantics and app coupling. Not benchmarked. |
| `nexrad_io` in FahrenheitResearch/bowecho | MIT OR Apache-2.0, app-workspace/path-coupled. Inspected `eea07fc0a032d4de439d0bfe2def9098afd8b5a8`, 2026-08-27. | Broader decoder, preview APIs and expanded-byte limits; unsafe optimizations, native libdeflate and explicit C bzip2-sys. | Useful independent reference, not the smallest library integration. Not benchmarked. |
| Own parser patterned after `radar-grib` | Our maintenance burden; no existing implementation to time. | Still needs Archive II signed record framing, message segmentation/offsets, variable Message 31 pointers, validation, moment geometry/statuses and sweep boundaries. GRIB packing code is not a NEXRAD decoder. | Reject unless a measured correctness/resource blocker cannot be solved in the library or adapter. |

The `nexrad` facade is unnecessary for ingest. The data crate owns files/decompression, decode owns messages/model conversion, and model owns common types. Its first stable release follows substantial API changes; avoid leaking those types into our manifest contract.

Sources: [workspace manifest](https://github.com/danielway/nexrad/blob/ae8b842ddfcb61fe7174db0e33789bcb5e3f03d2/Cargo.toml), [1.0 changelog](https://github.com/danielway/nexrad/blob/ae8b842ddfcb61fe7174db0e33789bcb5e3f03d2/nexrad-decode/CHANGELOG.md), [data API](https://docs.rs/nexrad-data/1.0.0/nexrad_data/), [decode API](https://docs.rs/nexrad-decode/1.0.0/nexrad_decode/), [rustdar parser](https://github.com/FahrenheitResearch/rustdar/blob/3a708877b3f7bb5a93e0d14a95df36a3e0ada698/crates/rustdar-core/src/level2.rs), [BowEcho decoder](https://github.com/FahrenheitResearch/bowecho/blob/eea07fc0a032d4de439d0bfe2def9098afd8b5a8/crates/nexrad_io/src/lib.rs), [BowEcho dependencies](https://github.com/FahrenheitResearch/bowecho/blob/eea07fc0a032d4de439d0bfe2def9098afd8b5a8/crates/nexrad_io/Cargo.toml).

## Real KGRR measurement

Anonymous HTTPS [bucket listing](https://unidata-nexrad-level2.s3.amazonaws.com/?list-type=2&prefix=2026/11/07/KGRR/&max-keys=5) returned [KGRR20261107_000118_V06](https://unidata-nexrad-level2.s3.amazonaws.com/2026/11/07/KGRR/KGRR20261107_000118_V06).

- Size: **9,438,864 bytes**.
- SHA-256: `672a54df400265b9bf5dce87b6db2aec22fa72e53c508374f215c2b759af17d6`.
- Header: KGRR, 2026-11-07T00:01:18.445Z. **67 records, 14 sweeps, 7,920 radials**.
- Published registry crates, not local source dependencies. Resolved `bzip2 0.6.1` and `libbz2-rs-sys 0.2.6`.
- Release build, x86_64 shared Hermes Kubernetes pod, two compiler jobs, no `parallel` feature. Host exposed 30 logical CPUs; this is not a dedicated machine or a CPU capacity measurement.
- Six repeated `File::scan()` calls on already-loaded compressed bytes. Timing includes bzip2 record decompression, message parsing and full model allocation. Excludes HTTP, disk read, gate iteration, QC, projection and tiles. Exclude first call from the warm median.

### Rust 1.97.0 results

Both release compilation and execution passed on the project's pinned toolchain.

| Call | Decode ms | Iterate REF/ZDR/RHO/PHI ms |
| --- | ---: | ---: |
| First | 1136.463 | 44.730 |
| Warm 1 | 2221.779 | 158.603 |
| Warm 2 | 1223.076 | 45.640 |
| Warm 3 | 972.616 | 38.478 |
| Warm 4 | 975.266 | 39.610 |
| Warm 5 | 1108.046 | 36.808 |

**Warm median: 1,108.046 ms; min 972.616 ms; max 2,221.779 ms.** Shared-host variance is substantial. An additional Rust 1.99.1 build/run passed, with warm median 894.147 ms (868.720–921.331 ms). Do not infer compiler speed differences from separate shared-host runs.

Every run on both toolchains returned identical counts and numeric sums:

| Moment | Gates | Numeric gates | Sum of numeric values |
| --- | ---: | ---: | ---: |
| REF | 10977120 | 2675091 | 11985809.5 |
| ZDR | 5613120 | 1509363 | 220047.34375 |
| RHO | 5613120 | 1509363 | 1224330.9018092304 |
| PHI | 5613120 | 1509363 | 141009558.75154692 |

The probe iterated each moment's `iter()` and accumulated only `MomentValue::Value`; below-threshold/range-folded statuses did not become numeric zeroes. These are reproducibility checks, not external value-parity validation.

No competing decoder or own-parser benchmark was run. This does not establish the fastest decoder. Peak RSS, precipitation-mode storm scans, selective-low-sweep decode and full cold-request latency remain unmeasured.

### Reproduce

Create a scratch Cargo project with the dependency above and a direct `nexrad-model = { version = "=1.0.0", default-features = false }` dependency. Download the volume and verify its hash:

```sh
curl -fL https://unidata-nexrad-level2.s3.amazonaws.com/2026/11/07/KGRR/KGRR20261107_000118_V06 -o KGRR20261107_000118_V06
sha256sum KGRR20261107_000118_V06
cargo +1.97.0 build --release -j 2
./target/release/level2-decoder-probe KGRR20261107_000118_V06
```

Core benchmark loop, after reading the file once:

```rust
let file = nexrad_data::volume::File::new(bytes);
for _ in 0..6 {
    let start = std::time::Instant::now();
    let scan = std::hint::black_box(file.scan()?);
    println!("decode_ms={}", start.elapsed().as_secs_f64() * 1000.0);
    for sweep in scan.sweeps() {
        for ray in sweep.radials() {
            for moment in [ray.reflectivity(), ray.differential_reflectivity(),
                ray.correlation_coefficient(), ray.differential_phase()]
                .into_iter().flatten()
            {
                let numeric_sum: f64 = moment.iter().filter_map(|v| match v {
                    nexrad_model::data::MomentValue::Value(v) => Some(f64::from(v)),
                    _ => None,
                }).sum();
                std::hint::black_box(numeric_sum);
            }
        }
    }
}
```

Execution logs for this research run: `/opt/data/cache/scratch/level2-decoder-build-197.log` and `/opt/data/cache/scratch/level2-decoder-benchmark-197.log`. Probe source: `/opt/data/cache/scratch/level2-decoder-probe/src/main.rs`. Scratch paths are temporary; the measurements and reproduction method above are the durable record.

## Integration and safety requirements

1. Keep an owned numeric polar representation in a dedicated adapter, not library types in the manifest wire format. Preserve station, per-ray time/azimuth/elevation, per-moment first gate/spacing and quality/status masks. Retain REF plus dual-pol fields needed by the north star; velocity UI remains out of scope.
2. The library explicitly maps REF/ZDR/PHI/RHO and preserves header scale/offset with 8/16-bit gates. Keep below-threshold and range-folded separate through QC. [Mapping source](https://github.com/danielway/nexrad/blob/ae8b842ddfcb61fe7174db0e33789bcb5e3f03d2/nexrad-decode/src/messages/digital_radar_data/data_block_id.rs), [moment model](https://github.com/danielway/nexrad/blob/ae8b842ddfcb61fe7174db0e33789bcb5e3f03d2/nexrad-model/src/data/moment.rs).
3. Bound compressed input, expanded bytes per record/volume, radials/gates, concurrency and cache bytes. `Record::decompress()` uses unrestricted `read_to_end`; stock `File::scan()` alone is not an expansion limit. Use a capped bzip2 reader in our adapter plus `Record::new()`/decode APIs, or upstream a small hardening change. A compressed-size cap is insufficient. [Record source](https://github.com/danielway/nexrad/blob/ae8b842ddfcb61fe7174db0e33789bcb5e3f03d2/nexrad-data/src/volume/record.rs).
4. Start fail-closed: malformed/incomplete input must not replace last-good imagery. `File::scan()` propagates the first error. Low-level iterators can keep earlier messages but do not establish a complete sweep. Define completeness before partial publication. Preserve split-cut/SAILS/MRLE repeated low tilts rather than grouping only by angle. [File source](https://github.com/danielway/nexrad/blob/ae8b842ddfcb61fe7174db0e33789bcb5e3f03d2/nexrad-data/src/volume/file.rs), [sweep selection](https://github.com/danielway/nexrad/blob/ae8b842ddfcb61fe7174db0e33789bcb5e3f03d2/nexrad-model/src/data/scan.rs).
5. Follow `.claude/commands/rust-dev.md`: Rust 1.97.0, fmt/clippy/tests and Temporal replay checks for implementation changes. Run decode on a bounded blocking worker/activity, heartbeat from async code, and deduplicate station/volume work. Timeout does not terminate `spawn_blocking`. Do not put decode in the manifest handler or parallelize calls to the lock-taking tiler on one Rayon pool.
6. `radar-tiler` consumes `radar_geo::Grid`, not polar rays. Decoder choice does not implement QC, beam-height/range masks, polar projection or mosaicking. The current `radar-api/src/manifest.rs` serves a 15-second ETag snapshot and last-good fallback; only advertise ready products. Server-only ingest serves both clients, but the build still needs web and app cross-fade/fallback and provenance UI.

The inspected `nexrad-decode` crate declares `forbid(unsafe_code)`. This is **not a transitive no-unsafe guarantee**. Rust compression and zerocopy dependencies can use unsafe internally. Pure Rust does not mean an unsafe-free dependency graph. No whole-graph safety audit was run.

Repo context read: `AGENTS.md`, `docs/reference/north-star.md`, `.claude/commands/rust-dev.md`, `radar-grib/src/lib.rs`, `radar-tiler/README.md`, and `radar-api/src/manifest.rs`. The recommendation preserves the map's on-demand viewed-radar scope, short cache and MRMS fallback. It does not choose cache TTL, storage schema or client thresholds.

## Remaining acceptance gates

- Compare values and geometry against an independent decoder and NOAA fixtures. Include 8/16-bit moments, split cuts/SAILS, corrupt/truncated records and storm-mode scans. [NOAA ROC ICD catalogue](https://www.roc.noaa.gov/interface-control-documents.php) lists Archive II 2620010J and RDA/RPG 2620002AA as released, and Build 25 documents as drafts. Decode success is not scientific QC validation.
- Measure memory, capped decompression, low-sweep selection and concurrent cold-request latency on deployment hardware. Roughly one second for this one volume is promising for an on-demand cache miss, not proof of the API/tile SLOs.
- Preserve publish-before-advertise and both-client parity in the implementation plan. No product code changed here; full radar-ng workspace tests were not run. The isolated published-crate release probe compiled and executed successfully on Rust 1.97.0 and 1.99.1.
