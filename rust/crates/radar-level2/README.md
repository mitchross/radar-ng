# Level II numeric and tile tracer

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

## Tile tracer (slice B)

[Issue #289](https://github.com/mitchross/radar-ng/issues/289) adds one requested z8–13 tile from a verified artifact.
`LEVEL2_TILE_ENABLED` (default false) gates it, on top of the numeric gate. No route, schedule, worker role or client uses it.

```text
verified commit -> artifact::load (checksums, REF + QC + geometry) -> hot sweep (PostgreSQL-reserved)
  -> blocking render (2 permits) -> class model shared with MRMS -> palette PNG
  -> immutable RustFS create + read-back -> PostgreSQL tile index -> hot tile cache
```

[polar.rs](src/polar.rs) maps each output pixel to a gate. Pixel centres come from global XYZ pixels, so tiles share seams.
Vincenty gives WGS84 ground distance and azimuth from the site. The 4/3-Earth formula in the design gives slant range per ray tilt.
The nearest ray owns the pixel and the nearest gate owns the range. A gap wider than 1.5 ray spacings is never bridged.
Missing support is outside the gates, a gap, beyond 150 km ground range or a folded, blanked or unobserved gate.
Below-threshold gates are observed clear air.

Zooms 8 and 9 (pixels over 125 m) average four sub-pixel samples as linear Z over supported samples only.
Missing never counts as clear. A weak feature under a quarter pixel can fall between the four samples.
Zooms 10–13 take one nearest-gate sample. There is no pyramid, no smoothing and no dBZ or colour averaging.
`radar_tiler`'s reflectivity table, class model and PNG encoder produce classic, muted and vivid exactly as MRMS does.

Admission uses these PostgreSQL tables beside the slice A ones:

| Table | Rule |
|---|---|
| `reservations` kind `sweep` | Resident sweeps plus the two decode slots stay within 1 GiB pool-wide |
| `reservations` kind `render` | In-flight scratch plus cached PNGs stay within 512 MiB pool-wide |
| `renders` | Two render permits, separate from decode permits |
| `tiles` | Key, SHA-256, bytes and transparency of each written tile |

A sweep load reserves three times its needed object bytes plus 1 MiB, then shrinks to its real retained size.
A render reserves 1 MiB of scratch, then shrinks to its PNG size when the PNG joins the hot cache.
A full budget evicts this process's idle entries, then returns MRMS. Nothing queues.
A resident sweep blocks the second concurrent decode until it is evicted, because both draw on the same 1 GiB.
The request task owns its permits. A timeout or dropped caller sets a cancel flag that the blocking loop checks per row.
The permits and reservations release only after the blocking job ends. Concurrent identical requests share one render.
A crash leaves reservations occupied, as in slice A. Tile objects are never overwritten; a differing write fails.

```sh
LEVEL2_NUMERIC_ENABLED=true LEVEL2_TILE_ENABLED=true radar level2-numeric --zoom 9 --x 134 --y 188 --palette classic
LEVEL2_NUMERIC_ENABLED=true LEVEL2_TILE_ENABLED=true radar level2-numeric --tile-only --zoom 13 --x 2152 --y 3010
```

`--tile-only` renders from the latest indexed artifact without network acquisition.
Measurements and acceptance mapping: [level2-tile-evidence.md](../../../docs/develop/level2-tile-evidence.md).

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
pool's 1 GiB working budget. Caller timeouts report fallback, then drain the owning task before CLI shutdown.
Reservations release only after its work ends.
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
Check angular progression and a single full rotation, including the closing gap. Preserve missing sectors.
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
Cached artifacts require fresh health, another verified object read and their own end age within 600 seconds.
Recheck age after publication and read-back. Reject future sweeps without replacing the last-good low-sweep clock.
Reseed stalled circular-volume hints from a bounded archive header, then retry live chunks before archive recovery.

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

No pilot, deployment-hardware capacity, client behavior or public enablement is claimed.
