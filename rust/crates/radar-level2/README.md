# Level II numeric tracer

Slice A of [issue #288](https://github.com/mitchross/radar-ng/issues/288) implements the
[approved design](../../../docs/develop/design/level2-close-up.md)'s private numeric path.
It adds no HTTP routes, schedules, worker roles, tiles, client code or deployment changes.
`radar level2-numeric` refuses work unless `LEVEL2_NUMERIC_ENABLED=true`. The default is false.

```text
bounded KGRR tile demand -> checked catalogue -> PostgreSQL admission
  -> anonymous chunks, then bounded archive recovery -> decode and conservative QC
  -> immutable RustFS Zarr objects -> checksum/read-back -> commit -> PostgreSQL index
```

The request supplies a centre and one valid z8–13 XYZ. Only KGRR is enabled for this tracer.
Catalogue selection bounds candidates to three within 150 km and breaks distance ties by station ID.
The feed-capable allowlist includes KGRR, KDTX, KLOT and KAPX, verified with anonymous chunk listings.
Adding a catalogue station does not enable it. Both clients continue using MRMS.

## Storage and admission

Set these variables only for a dedicated test database/schema and RustFS bucket/prefix:

| Variable | Meaning |
|---|---|
| `LEVEL2_NUMERIC_ENABLED` | Explicit private tracer gate, default false |
| `LEVEL2_PG_URL` | PostgreSQL connection string; use loopback or a trusted private transport |
| `LEVEL2_PG_SCHEMA` | Shared pool namespace, default `level2_numeric`; tests use unique `level2_test_*` schemas |
| `LEVEL2_S3_ENDPOINT`, `LEVEL2_S3_BUCKET` | RustFS endpoint and existing dedicated bucket |
| `LEVEL2_S3_ACCESS_KEY`, `LEVEL2_S3_SECRET_KEY` | Credentials supplied externally |
| `LEVEL2_OBJECT_PREFIX` | Dedicated prefix beginning `level2/`, for example `level2/issue288` |

Initialize only this schema when the gated CLI runs. PostgreSQL stores catalogue/health metadata,
station demand leases, decode reservations, object-byte reservations and sweep references.
No gate arrays enter PostgreSQL or Temporal. API replicas never read producer files or run this path.

Station admission serializes across connections and expires idle demand after 600 seconds.
Busy jobs remain active past demand expiry. Two persistent decode reservations also reserve the
pool's 1 GiB working budget. Caller timeouts never release a running job's reservation.
A process crash leaves reservations occupied rather than risk admitting duplicate blocking work.
Automatic orphan recovery, worker dispatch and deployment belong to later slices.

Hard limits are 32 MiB compressed per volume, 8 MiB expanded per record, 256 MiB expanded per
volume, 2048 rays per sweep, 4096 gates per moment and 32 cuts per volume.
Each job reserves 512 MiB, including input, record scratch, retained arrays and artifact copies.
Array admission leaves space for publication copies and metadata. Over-budget work records its
reason and leaves MRMS unchanged. There is no retained worker cache in A.

Object reservations include failed staging writes and stop at 10 GiB. A never deletes objects or
reclaims reservations. Only six sweeps per station remain indexed. Older metadata remains for
collision checks. Retention and pin-aware garbage collection belong to later slices.

## Numeric contract

The adapter pins `nexrad-data` and `nexrad-decode` to 1.0.0 with default features disabled.
It uses borrowed Message 31 blocks and its own capped bzip2 reader. It accepts modern AR2V
length-prefixed bzip2 records. Unsupported legacy formats fail closed.

Require contiguous chunk/ray numbers, a real cut start and end, and the commanded full ray count.
Preserve split cuts and supplemental cuts by volume, ordinal and actual acquisition times.
Subsequent demands discover cuts newer than the indexed end time. Stop fetching a selected cut
when its end is decoded. Archive recovery cannot outrank an available live cut.

Keep REF, RHO and ZDR with each ray's geometry, encoding parameters, quality flags and raw bytes.
Below-threshold, folded and blanked gates have separate QC bits and NaN physical values.
Absent moments have no gates, represented by equal adjacent offsets and null geometry.
No CC/ZDR/texture filter removes valid REF. Retain decoded site coordinates and raw height fields.
Cross-check coordinates against the catalogue. Antenna MSL altitude remains unknown.

Zarr v2 uses one-dimensional ragged moment arrays, gate offsets and at most 65,536 elements per
chunk. Physical values are little-endian float32. QC is uint8. Raw encoded bytes retain source
8/16-bit encoding. Ray time, azimuth and elevation are separate arrays. `.zattrs` preserves
per-ray/per-moment geometry and missing sectors. `commit.json` lists every object's SHA-256.
Conditional creates prevent overwrites. Write and read back every object, write the commit last,
verify the complete inventory, then index it. A numeric artifact is never a public ready frame.

Health separates latest chunk progress, completed low-sweep end time and access failures.
Eligibility includes exactly 600 seconds and rejects future observations. Higher tilts cannot
refresh the low-sweep clock. Feed progress warns after 120 seconds. Inactive sites are unknown.
Cached artifacts require both fresh health and another verified object read.

## Checks and reproduction

Run the [workspace gates](../../README.md#build-and-test) after sourcing `/opt/data/tools/radar-env.sh`.
`LEVEL2_TEST_PG` enables isolated PostgreSQL tests. Without it those tests explicitly print a skip.
The ignored `real_kgrr_archive_recovery_when_chunk_access_fails` test requires the PostgreSQL and
RustFS variables above. It blocks chunk access, then uses the real anonymous archive and RustFS.

```sh
cargo test -p radar-level2
cargo test -p radar-level2 real_kgrr_archive_recovery -- --ignored --nocapture
cargo build -p radar-level2 --examples
cargo run -p radar-level2 --example trace
```

The `trace` example calls the same implementation as `radar level2-numeric`.
[corpus.json](testdata/corpus.json) pins four real NOAA volumes by URL, byte count and SHA-256.
Download them to scratch. Install `arm_pyart==2.3.0` and `numpy==2.5.4` in a scratch virtualenv.
The [independent comparison](../../scripts/level2-parity.py) loads Py-ART's standalone Level II reader.

```sh
python rust/scripts/level2-parity.py --inspect rust/target/debug/examples/inspect /scratch/KGRR20261107_000118_V06 /scratch/KCRP20170826_044114_V06 /scratch/KDMX20220305_232324_V06 /scratch/KDMX20250314_175512_V06
```

The corpus covers VCP 35 and 212, Hurricane Harvey, severe KDMX weather, supplemental/split low
cuts, 8/16-bit moments and absent RHO/ZDR. Compare every retained gate/status, encoding field,
ray time and geometry. Synthetic decoder tests cover corruption, truncation, bombs and cap boundaries.
The corruption/bomb checks reject both readers. Py-ART accepts the truncated input partially;
the bounded adapter rejects it. A historical KCRP volume header follows its first radial by 2.132 seconds; retain both source clocks.
Do not infer an observation time from the volume filename or header.

No pilot, deployment-hardware capacity, tile geometry, rendering, client behavior or public enablement is claimed.
