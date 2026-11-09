# Level II access: use live chunks, keep the archive as fallback

Research for [Where and how fast can we get Level II volumes?](https://github.com/mitchross/radar-ng/issues/273), under [Level II close-up reflectivity](https://github.com/mitchross/radar-ng/issues/272).

Checked on 2026-11-09 UTC (2026-11-08 in Detroit). Repo baseline: `d78b02ab49e48ba57ee4ee29af476675f4085fb4`. This is a decision note, not product code or a production latency SLO.

## Decision

Fetch `unidata-nexrad-level2-chunks` anonymously over HTTPS on the server for viewed stations. Assemble a complete lowest reflectivity sweep, including its dual-pol fields, without waiting for the entire volume. Keep `unidata-nexrad-level2` for bootstrap, recent history and gap recovery. Do not use the deprecated `noaa-nexrad-level2` bucket. Start with bounded polling; SNS is optional and does not meet a strict no-account/no-credentials requirement.

This distinction matters: in the measured KGRR volume, the lowest sweep arrived **439 seconds before the full archive volume**, using 1,430,000 instead of 8,449,360 compressed bytes. The archive itself was not slow after the scan finished; it necessarily waited for the remaining elevations.

## Sources and access

| Source | Access and shape | Freshness and role |
| --- | --- | --- |
| `unidata-nexrad-level2-chunks` | Public unsigned LIST/GET, `us-east-1`. `SITE/volume/YYYYMMDD-HHMMSS-NNN-S`, `-I`, `-E`. Bzip2 records, not standalone full volumes. | Streaming acquisition. Use for current lowest sweeps. No numeric publication SLA or retention guarantee found. |
| `unidata-nexrad-level2` | Public unsigned LIST/GET, `us-east-1`. `YYYY/MM/DD/SITE/SITEYYYYMMDD_HHMMSS_V06` for the current samples. Complete volume; `_MDM` objects also occur and must not be treated as reflectivity volumes. | Historical archive plus newly completed volumes. Good recovery source; cannot expose an unfinished scan. |
| `noaa-nexrad-level2` | Old archive name. Both current-day and prior-day KGRR anonymous listings returned HTTP 403. | Deprecated, not a fallback. Registry says unavailable starting September 1, 2025. |
| Archive SNS | `arn:aws:sns:us-east-1:684042711724:NewNEXRADLevel2Archive`. | New archive-object notifications; no earlier sweep access than the archive. |
| Chunk SNS | Registry HTML advertises `arn:aws:sns:us-east-1:684042711724:NewNEXRADLevel2ObjectFilterable`. Fields include SiteID, VolumeID, ChunkID, ChunkType, DateTime and L2Version. | Optional filtered station notifications. Current raw README/registry YAML mask the suffix as `***`; verify the exact topic and permissions during an authenticated setup. Subscription was not tested. |

Primary sources: [AWS Registry](https://registry.opendata.aws/noaa-nexrad/), [registry source and migration notice](https://github.com/awslabs/open-data-registry/blob/main/datasets/noaa-nexrad.yaml), [Unidata migration announcement](https://www.unidata.ucar.edu/blogs/news/entry/important-changes-to-noaa-nexrad), [AWS layout and notification documentation](https://github.com/awslabs/open-data-docs/blob/main/docs/noaa/noaa-nexrad/README.md).

**Documentation trap:** the layout README still names the retired archive bucket and archive SNS account `811054952067`. Unidata's migration notice and the current Registry supersede those names. Historical `.gz` examples also do not describe the modern `_V06` sample's framing.

S3 reads need no account, key or SDK. SNS subscription setup requires an AWS account and authenticated permissions; SQS consumption adds authenticated queue access. An HTTPS receiver can run outside AWS, but that does not make subscribing anonymous. See [SNS setup](https://docs.aws.amazon.com/sns/latest/dg/sns-setting-up.html) and [Subscribe API](https://docs.aws.amazon.com/sns/latest/api/API_Subscribe.html). Do not make a subscription a prerequisite for self-hosters.

## Live KGRR evidence

All requests used Python standard-library `urllib.request` without Authorization or AWS credentials. The listing probe began at `2026-11-09T03:07:43.910138Z`; requests ran sequentially, so counts describe their individual snapshots, not one atomic instant. Both date prefixes were checked to cover UTC today and Mitch's local today.

| Archive prefix | Objects | `_V06` volumes | `_MDM` objects | Volume start → object LastModified, seconds (min / median / max) |
| --- | ---: | ---: | ---: | --- |
| `2026/11/08/KGRR/` | 192 | 169 | 23 | 486 / 498 / 515 |
| `2026/11/09/KGRR/` | 25 | 21 | 4 | 511 / 512 / 515 |

The November 8 volumes ranged from 2,120,834 to 8,220,563 bytes. The early November 9 volumes ranged from 8,275,309 to 8,519,393 bytes. Do not hard-code a five-minute cadence: the current sample took roughly eight and a half minutes per full volume.

A complete matched volume:

- Archive: `2026/11/09/KGRR/KGRR20261109_025422_V06`, 8,449,360 bytes, LastModified `03:02:54Z`.
- Chunk directory: `KGRR/471/`, 67 objects, summed size 8,449,360 bytes.
- Start: `20261109-025422-001-S`, 2,362 bytes, LastModified `02:54:25Z`. This contains metadata, not reflectivity radials.
- First radial chunk: `20261109-025422-002-I`, LastModified `02:54:36Z`. Its first radial time was `02:54:22.387Z`; its last radial time was `02:54:34.146Z`.
- Lowest sweep completion: chunk `007-I`, LastModified `02:55:35Z`. Its last radial was azimuth number 720, elevation number 1, angle `0.4833984375°`, radial status 2 (end of elevation), collected at `02:55:33.542Z`.
- Chunks `002`–`007` each contained 120 radials. Their moment blocks included `REF`, `ZDR`, `PHI`, `RHO` and `CFP`, plus volume/elevation/radial metadata.
- Final chunk: `067-E`, LastModified `03:02:54Z`. Its final radial was elevation 14, status 4 (end of volume), collected at `03:02:52.309Z`.

### What the latency numbers mean

For the six lowest-sweep radial chunks, **last contained radial → S3 LastModified** was 1.458–2.974 seconds, median 2.1655 seconds. Lowest-sweep completion was 73 seconds after the integer timestamp in the volume key. The archive appeared 512 seconds after that key timestamp, but only 1.691 seconds after the final decoded radial. These are different clocks and must not be reported as one latency.

LastModified is a storage timestamp, not a directly observed first-availability measurement. This was retrospective parsing of existing objects, not continuous arrival monitoring. It excludes polling, transfer, decode, QC, tile publication and manifest/client refresh time. It is a one-station snapshot, not a p95, severe-weather benchmark, or SLA. The format parser inspected headers and moment presence; it did not validate gate values or render a product.

## Can we fetch only the lowest tilt?

**Yes, as a parsed stream prefix; no, as an elevation-select object API.**

The [Archive II transfer ICD 2620010J](https://www.roc.noaa.gov/public-documents/icds/2620010J.pdf) describes the volume header, length-prefixed compressed records and LDM key format. The [RDA/RPG ICD 2620002AA](https://www.roc.noaa.gov/public-documents/icds/2620002AA.pdf) defines message 31 elevation, radial status and moment blocks. [Py-ART's reader source](https://github.com/ARM-DOE/pyart/blob/main/pyart/io/nexrad_level2.py) supplied an independent implementation cross-check for record stepping and header offsets.

The throwaway probe read the optional 24-byte volume header, then signed big-endian record lengths and independently bzip2-decompressed each record. It inspected message 31 headers and block pointers. In this volume, metadata plus six radial chunks completed the first sweep: **seven GETs and 1,430,000 bytes**, about **83.1% fewer bytes** than the full volume. Chunk `008-I` had elevation number 2 at approximately the same physical angle, with `REF/VEL/SW` but without the first sweep's dual-pol blocks. Thus elevation number and physical tilt are not interchangeable; split cuts and supplemental low scans need explicit treatment.

Never assume seven objects always suffice. `S/I/E` marks volume position, not elevation boundaries. Check sequence continuity, station, volume identity, radial status, azimuth coverage and required quality fields. Stop only after the selected sweep is complete. Missing chunks must not produce a seemingly complete frame.

The archive supports [S3 Range GET](https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObject.html). A real request for bytes `0-1429999` returned HTTP **206**, `Content-Range: bytes 0-1429999/8449360`; its body exactly matched concatenating chunks `001-S` through `007-I`. A 64-byte range also showed `AR2V0006.471`, the station header and bzip2 framing. This proves prefix retrieval for this modern volume, not arbitrary offset-to-tilt indexing or support for every historical format. Parse record lengths or fetch incrementally; arbitrary ranges may split a compressed record. Range retrieval saves bandwidth only after the archive exists.

## Server integration recommendation

1. Bootstrap a viewed station from the newest completed archive volume; read its header to learn the volume number. Inspect the current/next chunk directories, allowing wraparound and missed volumes. Cache discovery centrally and bound recovery scans. Do not rank circular volume directory names as timestamps.
2. Poll only active viewed stations with a bounded interval and backoff. A proposed 5–10 second polling interval trades requests for added freshness delay; it is a design choice, not a source guarantee. Follow exact chunk sequence keys and use complete-volume archive recovery for gaps. Coalesce concurrent viewport requests per station.
3. Download/decode the chosen reflectivity and QC sweep server-side. Retain source key, station, volume start, actual sweep times and elevation. Publish an immutable complete frame only after tiles exist. Keep MRMS when Level II is incomplete or late.
4. Expose only radar-ng endpoints to web and app. Keep AWS URLs and chunk mechanics out of clients. Add SNS later only as an optional discovery accelerator; still reconcile listings after missed notifications.

Repo context checked: `AGENTS.md`, `.claude/commands/rust-dev.md`, `.claude/commands/add-layer.md`, `docs/reference/north-star.md`, `rust/crates/radar-tiler/README.md` and `rust/crates/radar-api/src/manifest.rs`. The tiler consumes a grid, not a raw Level II stream, so decoding/QC precedes rendering. The current manifest route caches reads for 15 seconds and sets a 15-second HTTP max-age. Feed speed alone will not guarantee equivalent end-to-end freshness. Exact storage, metadata schema, decoder crate and rendering changes belong to the other map decisions, not this access ticket.

## Terms and remaining risks

The [Registry's NOAA terms](https://registry.opendata.aws/noaa-nexrad/) permit public use as desired. Attribute NOAA/NEXRAD, do not imply NOAA endorsement, and label rendered/QC-modified output as processed data rather than original unaltered NOAA data. Public source access does not eliminate our bandwidth, compute or optional AWS service costs.

Before enabling a pilot, measure multiple stations/VCPs and busy weather continuously, including first availability, sweep completion, missing/out-of-order chunks, supplemental scans and restart recovery. Establish a bounded chunk-retention assumption through monitoring or provider confirmation; the docs do not promise it. Do not treat bucket availability as station health. Keep freshness/QC gates and MRMS fallback independent of this source choice.

SNS was documented but not subscribed to. No decoder crate was benchmarked, no reflectivity gates or QC were validated, and no product tests were run because this branch changes only this research note.

## Reproduce the access checks

These requests are anonymous and return XML listings or the specified binary prefix. Listings change as new scans arrive.

```sh
curl -fsS 'https://unidata-nexrad-level2.s3.amazonaws.com/?list-type=2&prefix=2026/11/08/KGRR/'
curl -fsS 'https://unidata-nexrad-level2.s3.amazonaws.com/?list-type=2&prefix=2026/11/09/KGRR/'
curl -fsS 'https://unidata-nexrad-level2-chunks.s3.amazonaws.com/?list-type=2&prefix=KGRR/471/'
curl -fsS -H 'Range: bytes=0-63' 'https://unidata-nexrad-level2.s3.amazonaws.com/2026/11/09/KGRR/KGRR20261109_025422_V06'
```

The probe handled `NextContinuationToken` for full listings and filtered `_V06` separately from `_MDM`. Local scratch evidence: `/opt/data/cache/scratch/level2-access-evidence/listings.json`, `sample.json`, and the `level2-access-{probe,sample,metrics,range}.py` probes. Scratch files are ephemeral; the observations and method above are the durable record.
