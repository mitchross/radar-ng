# Level II QC review on archive hazard cases (#293)

This report runs the shipped Level II QC on real NOAA archive volumes, one or more per hazard type. It also covers volumes the archive path rejects. Mitch signs off at the end.

The QC measured is the code on `feat/storm-sweep-finish` at `d96e36f`. That code is the `qc_values` sentinel and blanking masks in [`decode.rs`](../../rust/crates/radar-level2/src/decode.rs), followed by the artifact and the sampler. If later work changes the QC, rerun the script; the numbers below will move.

## How to run

```sh
cd rust
scripts/level2-qc-report.sh /tmp/level2-qc-report
```

The script works through the case list in [`hazard-cases.json`](../../rust/crates/radar-level2/testdata/hazard-cases.json):

1. Downloads each case from `unidata-nexrad-level2` into `LEVEL2_ARCHIVE_DIR` (default `$TMPDIR/level2-archive`).
2. Checks byte count and sha256, and unpacks `.gz` objects.
3. Runs [`examples/qc_review.rs`](../../rust/crates/radar-level2/examples/qc_review.rs).

Archive data and panels are never committed. A run takes about 5 minutes on one core plus a one-time ~95 MB download. It writes:

- `metrics.json` for each case
- `summary.json`, plus `summary.md`, which holds the table below
- four 1000×1000 PNG panels per case, at 300 m per pixel and ±150 km from the radar

The example exits non-zero if a hard check fails. `--screen --file STATION:PATH` gives gate counts only, which is useful for vetting candidate volumes.

For each complete lowest sweep, the generator decodes the volume the way the feed does (`Decoder::push`, then `finish`). It then builds and publishes the artifact into an in-memory store, loads it back, and samples it with `PolarSweep::sample` and `sample_tile`.

## Cases

Bytes and sha256 are of the archive object as downloaded on 2026-11-16. The full records, including the reason each case was picked and its source, are in the JSON.

| Category | Case | Archive key | Event source | Archive path |
|---|---|---|---|---|
| Hail / low CC | `kewx-2021-04-29-hail` | `2021/04/29/KEWX/KEWX20210429_004538_V06` | [SPC 2021-04-28](https://www.spc.noaa.gov/climo/reports/210428_rpts_filtered.html): 4.00 in hail at Hondo, 0045Z | published, 8 lowest sweeps |
| Tornado debris | `kdmx-2022-03-05-tornado` | `2022/03/05/KDMX/KDMX20220305_223552_V06` | [SPC 2022-03-05](https://www.spc.noaa.gov/climo/reports/220305_rpts_filtered.html): "tornado ongoing with debris" at Winterset, 2235Z | published, 4 |
| Tornado debris (legacy) | `ksgf-2011-05-22-joplin` | `2011/05/22/KSGF/KSGF20110522_224348_V03.gz` | [SPC 2011-05-22](https://www.spc.noaa.gov/climo/reports/110522_rpts.html): Joplin EF-5, 2241Z | rejected: `invalid record length` |
| Light rain | `kdtx-2025-03-16-light-rain` | `2025/03/16/KDTX/KDTX20250316_031344_V06` | IEM ASOS KDTW `-RA` 02:53Z and 03:53Z | published, 2 |
| Light snow | `kdtx-2025-01-02-light-snow` | `2025/01/02/KDTX/KDTX20250102_061811_V06` | IEM ASOS KDTW `-SN` 05:53Z to 07:53Z, 29-30 F | published, 2 |
| Melting layer | `kdtx-2025-01-31-melting-layer` | `2025/01/31/KDTX/KDTX20250131_171657_V06` | IEM ASOS KDTW `-RA BR`; KDTX 12Z RAOB 0 °C at ~2.3 km | published, 2 |
| Clear-air biology | `klsx-2025-09-26-birds` | `2025/09/26/KLSX/KLSX20250926_040226_V06` | [Cornell: record migration night, 25 Sep 2025](https://news.cornell.edu/stories/2025/09/record-breaking-night-bird-migration-detected-radar) | published, 2 |
| Clear-air biology | `kmkx-2025-09-26-birds` | `2025/09/26/KMKX/KMKX20250926_040347_V06` | same | published, 2 |
| AP | `klix-2020-05-11-ap` | `2020/05/11/KLIX/KLIX20200511_121334_V06` | [NWS New Orleans beam-ducting tweets, Mon 11 May 2020](https://www.foxnews.com/us/weather-radar-new-orleans-radar-beam-ducting-phenomenon-temperature-inversion) | published, 4 |
| Wind farm | `kmkx-2026-11-15-wind-farm` | `2026/11/15/KMKX/KMKX20261115_120409_V06` | [NWS MKX: Butler Ridge wind farm ~30 mi N of KMKX](https://www.weather.gov/mkx/windfarm) | published, 2 |
| Missing dual-pol | the second same-elevation sweep in every split-cut case above, plus `kgrr-2012-07-01-legacy` | `2012/07/01/KGRR/KGRR20120701_000708_V06.gz` | archive era only | split cuts published; KGRR rejected: `invalid record length` |
| Folded gates | the split-cut Doppler sweeps above (for example KDMX sweep 2: 26,731 folded gates) | | | published |
| Azimuth gaps | `kmpx-2024-07-10-azimuth` | `2024/07/10/KMPX/KMPX20240710_180457_V06` | found by screening | rejected: `duplicate or reversed azimuth` |
| Incomplete volume | `kmkx-2025-09-26-truncated` | `2025/09/26/KMKX/KMKX20250926_043857_V06` | found by screening | rejected: `incomplete sweep at EOF` (2 complete lowest sweeps measured anyway) |
| **Blanked gates** | missing | | | No radial with spot blanking turned up in 423 screened volumes: the first V06 volume of each hour for every K-site at 2026-11-15 12Z, 2025-05-20 00Z and 2024-07-10 18Z, plus every case above. |
| **Served azimuth gap** | missing | | | No decodable lowest sweep had `missing_sectors_deg`. The decoder requires 360/spacing radials, so real gaps showed up as rejected volumes. |

## Hard checks

Both checks passed on every complete lowest sweep: 30 sweeps from 10 volumes, including the 2 complete sweeps of the truncated KMKX object.

- **Valid REF is never lost only because dual-pol is missing.** For every sweep, the generator builds a second artifact with RHO and ZDR stripped from every ray.
  - Over the full 1000×1000 grid, the served state was identical with and without dual-pol: 0 mismatched pixels everywhere.
  - At the strongest gate, the z9 and z12 tiles were bit-identical.
  - No valid REF gate went unserved on a ray without dual-pol: 0 pixels.
  - The split-cut sweeps have REF on 720/720 rays and RHO on 0/720, and they serve exactly like the dual-pol sweeps.
- **No sentinel becomes a dBZ.**
  - Gate level: every gate with raw code 0 or 1, or with blanking set, decoded to NaN with a flag. Every other gate decoded to exactly `(code - offset) / scale`.
  - Served level: every served dBZ pixel traces back to a valid raw gate with the same value.
  - Both counts were 0 in every sweep.

These are coverage checks of the current code; they do not prove the QC is good. The findings below are about quality.

## Findings

1. **No non-meteorological echo is removed today.** The shipped QC masks only the code-0 and code-1 sentinels and blanked radials, so it masks no valid weather (0 masked valid gates in every case). It also leaves all clutter in place. The residual-clutter proxy counts retained REF ≥ 5 dBZ where RHO < 0.80. The share of RHO-checked gates in each case:
   - birds: 96% at KLSX and 88% at KMKX
   - AP at KLIX: 60%
   - wind-farm morning at KMKX: 60%
   - hail at KEWX: 20%
   - KDMX tornado: 9%
   - light rain, snow and the melting layer: 0.5-6%

   The KMKX wind-farm sweep's maximum (43.5 dBZ at 43.39 N, 88.49 W, 47 km north) is the Butler Ridge farm.
2. **A low-RHO filter would cut real hazards.** Low RHO also appears inside the KEWX hail core: 389 gates are ≥ 45 dBZ with RHO < 0.80 in sweep 1, and 242-389 across its four dual-pol sweeps. The same holds in the KDMX debris storm (27-47 gates ≥ 45 dBZ). A plain CC threshold would remove the hail core and the debris signature. Any clutter rule needs REF, texture or range context, and these two cases must stay in its tests.
3. **Split-cut Doppler sweeps are served with range-folded holes inside storms.** The feed publishes every complete lowest sweep. In VCP 212/215, that includes the Doppler cut at the same elevation, which has no dual-pol and has range-folded (code 1) REF. Counts per volume:

   | Case | Holes in Doppler cut | Holes containing folded gates | Holes in matching surveillance cut |
   |---|---|---|---|
   | KDMX | 178-226 | 171-216 | 6-11 |
   | KEWX | 47-106 | 38-88 | 2-4 |
   | KLIX | 105-123 | 42-48 | 38-64 |

   Mitch should decide whether the Doppler cut is served, merged, or skipped.
4. **Peak values survive.** At the strongest raw gate of every sweep, the served value equals the raw value (for example 71.0 dBZ at KEWX and 69.5 at KDMX). The z12 tile maximum there equals it as well. z9 tiles can be lower by up to 6 dB (for example KLSX sweep 2: 41.6 vs 47.5) because of the shipped z9 sampling. On the 300 m review grid, `served_grid_max_dbz` can also come in below the raw maximum where a single gate is narrower than a pixel. That is a grid-resolution effect, not QC.
5. **Whole volumes are rejected, so MRMS stays:**
   - Legacy volumes, both the 2011 V03 gzip and the 2012 V06 gzip: `invalid record length`.
   - A real volume with an irregular azimuth step (0.83° on a 0.5° cut at KMPX): `duplicate or reversed azimuth`.
   - A truncated archive object: `incomplete sweep at EOF`.

   The truncated KMKX object already has two complete lowest sweeps that pass both hard checks, but the archive path drops them.

## Per-sweep numbers

From `summary.md`, with these definitions:

- Gates are counted to 150 km ground range.
- *Low-RHO* counts retained REF with RHO < 0.80, binned by dBZ; the number in brackets is gates within 40 km.
- *Holes* are non-echo regions of at most 200 gates whose neighbours are all REF ≥ 15 dBZ. The bracket gives the gate count and the number of holes that contain a folded or blanked gate.

| Case | Sweep | El | RHO rays | Valid | Below | Folded | Blanked | Masked valid | Low-RHO 5-30 / 30-45 / ≥45 (≤40 km) | Raw max | Served max | z9 / z12 at core | Holes (gates, masked) | No sentinel | REF w/o dual-pol |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| kewx-hail | 1 | 0.53 | 720/720 | 143489 | 282751 | 0 | 0 | 0 | 9988 / 763 / 389 (8569) | 67.5 | 67.5 | 67.0 / 67.5 | 2 (5, 0) | pass | pass |
| kewx-hail | 2 | 0.53 | 0/720 | 141409 | 280563 | 4268 | 0 | 0 | – | 67.5 | 67.5 | 67.5 / 67.5 | 63 (91, 55) | pass | pass |
| kewx-hail | 10 | 0.48 | 720/720 | 146916 | 279324 | 0 | 0 | 0 | 11119 / 710 / 247 (9872) | 71.0 | 71.0 | 71.0 / 71.0 | 3 (8, 0) | pass | pass |
| kewx-hail | 18 | 0.48 | 0/720 | 145567 | 275446 | 5227 | 0 | 0 | – | 67.5 | 67.5 | 67.5 / 67.5 | 106 (189, 88) | pass | pass |
| kdmx-tornado | 1 | 0.52 | 720/720 | 171558 | 254682 | 0 | 0 | 0 | 9259 / 248 / 27 (6938) | 69.5 | 69.5 | 69.5 / 69.5 | 6 (17, 0) | pass | pass |
| kdmx-tornado | 2 | 0.53 | 0/720 | 124726 | 274783 | 26731 | 0 | 0 | – | 69.0 | 69.0 | 69.0 / 69.0 | 178 (437, 171) | pass | pass |
| kdtx-light-rain | 1 | 0.44 | 720/720 | 294270 | 131970 | 0 | 0 | 0 | 2423 / 7 / 0 (2220) | 41.5 | 41.5 | 41.5 / 41.5 | 28 (142, 0) | pass | pass |
| kdtx-light-snow | 1 | 0.48 | 720/720 | 73511 | 352729 | 0 | 0 | 0 | 407 / 0 / 0 (399) | 36.5 | 36.0 | 35.0 / 36.5 | 0 (0, 0) | pass | pass |
| kdtx-melting-layer | 1 | 0.53 | 720/720 | 328317 | 97923 | 0 | 0 | 0 | 1279 / 118 / 2 (1051) | 54.5 | 47.5 | 51.5 / 54.5 | 38 (187, 0) | pass | pass |
| klsx-birds | 1 | 0.44 | 720/720 | 425267 | 973 | 0 | 0 | 0 | 404883 / 1300 / 0 (95295) | 42.5 | 42.5 | 41.5 / 42.5 | 65 (271, 0) | pass | pass |
| kmkx-birds | 1 | 0.48 | 720/720 | 393988 | 32252 | 0 | 0 | 0 | 302804 / 117 / 1 (61699) | 48.5 | 48.5 | 48.5 / 48.5 | 10 (16, 0) | pass | pass |
| kmkx-truncated | 1 | 0.48 | 720/720 | 392374 | 33866 | 0 | 0 | 0 | 297606 / 138 / 7 (64343) | 48.5 | 48.5 | 48.5 / 48.5 | 16 (18, 0) | pass | pass |
| klix-ap | 1 | 0.44 | 720/720 | 234886 | 191354 | 0 | 0 | 0 | 56301 / 1924 / 29 (7695) | 70.5 | 70.5 | 69.3 / 70.5 | 64 (142, 0) | pass | pass |
| klix-ap | 8 | 0.44 | 0/720 | 208331 | 197562 | 20347 | 0 | 0 | – | 78.0 | 78.0 | 76.8 / 78.0 | 105 (172, 48) | pass | pass |
| kmkx-wind-farm | 1 | 0.49 | 720/720 | 301328 | 124912 | 0 | 0 | 0 | 24744 / 60 / 0 (20744) | 43.5 | 43.5 | 43.5 / 43.5 | 0 (0, 0) | pass | pass |

The other 15 sweeps are in the generated `summary.md`. All of them pass both hard checks.

## Pictures

Panels are in the run's output directory, `<case>/sweep<N>/`. They are not committed: 13 cases come to 8.5 MB. The reviewed run is at `/opt/data/cache/scratch/c5-report/` on the agent host; rerun the script to regenerate it anywhere. Panels are written for the first sweep with dual-pol and the first sweep without.

| Panel | Contents |
|---|---|
| `raw.png` | Raw REF in the shipped classic palette, using only the decoder's sentinel codes. Code 0 is dark grey and code 1 is purple. |
| `after-qc.png` | What `PolarSweep::sample` serves. Served "clear" is slate. Background colour means either no data or valid REF below the palette's first class. |
| `mask.png` | Why each gate is masked: dark grey below threshold, purple range-folded, light grey valid, orange blanked, red no ray. |
| `rho.png` | RHO for valid REF, on a blue-to-red ramp from 0.2 to 1.05. |

## Reviewer sign-off

Mitch fills this in. The generator does not decide pass or fail for quality.

| Case | Panels look right | Weather kept | Clutter acceptable | Notes | Sign-off |
|---|---|---|---|---|---|
| kewx-2021-04-29-hail | | | | | |
| kdmx-2022-03-05-tornado | | | | | |
| ksgf-2011-05-22-joplin | | | | | |
| kdtx-2025-03-16-light-rain | | | | | |
| kdtx-2025-01-02-light-snow | | | | | |
| kdtx-2025-01-31-melting-layer | | | | | |
| klsx-2025-09-26-birds | | | | | |
| kmkx-2025-09-26-birds | | | | | |
| kmkx-2025-09-26-truncated | | | | | |
| klix-2020-05-11-ap | | | | | |
| kmkx-2026-11-15-wind-farm | | | | | |
| kmpx-2024-07-10-azimuth | | | | | |
| kgrr-2012-07-01-legacy | | | | | |
| Split-cut serving (finding 3) | | | | | |
