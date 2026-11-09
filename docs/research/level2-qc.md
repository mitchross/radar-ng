# Level II reflectivity: cheap, conservative QC

Decision for [What cheap QC removes clutter before display?](https://github.com/mitchross/radar-ng/issues/276), under [Level II close-up reflectivity](https://github.com/mitchross/radar-ng/issues/272).

Researched 2026-11-08. Repo baseline: `d78b02ab49e48ba57ee4ee29af476675f4085fb4`.

## Recommendation

Ship a **conservative, versioned server QC profile**. Keep the radar's supplied clutter-filtered reflectivity; remove invalid/below-threshold samples, respect actual gate coverage and blanking, and retain suspicious-echo diagnostics. **Do not default to deleting reflectivity at any fixed correlation-coefficient (CC), ZDR, or texture threshold.** That is cheap computationally but unsafe scientifically.

This is not a promise of clutter-free precipitation. Residual AP, insects, birds, wind farms, and other non-weather echoes remain possible. If the pilot requires a precipitation-only view, keep it on MRMS until an additional classifier passes validation. Do not silently call a heuristic mask ground truth.

### Default profile: `level2-conservative-v1` (proposed)

| Operation | Default | Reason |
| --- | --- | --- |
| REF encoded value 0 | Missing/transparent; record `below_threshold` | Supplied signal is below the instrument threshold. |
| REF encoded value 1 | Non-numeric `range_folded`; transparent in reflectivity; retain reason | Never decode this as a real dBZ value or zero rain. |
| REF values 2 and above | Decode using each moment block's scale/offset | Do not use a fixed scale or a dBZ lower cutoff. |
| Broken lengths, nonfinite decoded REF, missing REF/radial | Exclude invalid gates; reject malformed structures | Do not fabricate zeros or complete coverage. |
| Spot-blanked radial / absent coverage | Exclude affected coverage | Keep acquisition geometry and quality distinct from dry weather. |
| Geometry | Use each moment's first gate, spacing, count, radial angle and time | REF and dual-pol arrays need not be interchangeable by index. |
| CC / ZDR / PHI / CFP absent or invalid | Mark quality unavailable; keep valid REF | Missing dual-pol is not proof of clutter. |
| CC below 0.8 | Diagnostic `low_cc_candidate` only, when CC is valid and collocated | This is an interpretation aid, not a trustworthy delete rule. |
| ZDR threshold; texture; despeckling | Off | Preserve hail, weak snow, small cores and sharp storm boundaries. |
| CFP / clutter maps | Retain diagnostic metadata if available; no extra reflectivity subtraction or blanket mask | Filter activity does not establish that surviving signal is clutter. |
| Range / height | Hard-mask only unobserved or explicitly invalid coverage | Distance and beam height affect usefulness, not whether an echo is real. |

The final close-up range cap and MRMS cross-fade belong to the rendering/build spec, not an invented universal QC range. Compute beam-height awareness once per sweep geometry and publish station altitude, elevation and range metadata. Do not treat a fixed-height cut as a surface-rain classifier. Terrain blockage and anomalous propagation need more information than a distance cutoff.

Keep a reason bitset separate from the REF values. Apply masks **before** gridding/interpolation, preserve no-data boundaries, and prevent interpolation from filling missing azimuths or masked gaps. Decode and QC once per station/scan/tilt/profile cache miss, not for every tile, palette, API replica or viewer. Include the QC profile and its parameters in immutable frame/cache identity.

## Evidence: what is in Level II

The NOAA ROC [current ICD index](https://www.roc.noaa.gov/interface-control-documents.php) lists Archive II Build 23 revision J and RDA/RPG Build 24 revision AA as published documents; newer Build 25 documents are marked draft. This research uses the published specifications.

1. [RDA/RPG ICD 2620002AA](https://www.roc.noaa.gov/public-documents/icds/2620002AA.pdf), Table XVII-B and notes 15, 20–21 (printed pp. 3-84–3-90): moment headers carry gate count, first range, spacing, word size, scale and offset. `F = (N - OFFSET) / SCALE`; use the supplied values. REF/RHO/ZDR/PHI and other ordinary moments reserve 0 for below-threshold and 1 for range-folded data. Actual data start at 2. **Do not apply these ordinary-moment sentinel meanings to CFP.**
2. The same table's SNR threshold is a moment-header parameter in dB, not an array of measured per-gate SNR. Do not compare decoded dBZ to that number or assume an NCP/SNR field exists just because a generic library supports one. Respect supplied thresholding rather than attempting a new SNR cut without the required measurements.
3. Table XVII-I, notes 29–31: **CFP means Clutter Filter Power Removed**, not a categorical ground-clutter label. Codes 0/1/2 mean filter not applied / point clutter filter applied / dual-pol filtered but not single-pol moments. Codes 3–7 are reserved; numeric CFP begins at 8. The moment describes filtered versus unfiltered reflectivity. It is not a second correction to subtract from REF, nor a reason to erase all gates where filtering happened.
4. Table XVII-A and note 8 describe spot blanking; note 11 allows absent moment pointers; note 12 makes moment presence VCP-dependent. Absence must not erase valid reflectivity from the same radial.
5. [Archive II ICD 2620010J](https://www.roc.noaa.gov/public-documents/icds/2620010J.pdf), sections 7.5–7.7: message 15 carries the clutter map. Message 13's bypass map **has not been included since Build 19**; its former space is zero-filled. A legacy parser exposing a bypass-map type is not evidence that current files supply a usable map. A static map indicates filter geography, not whether today's gate contains weather. Do not require these maps for the first display profile.

These specifications establish fields and semantics, not their prevalence in a selected decoder or every current public scan. Verify optional CFP and message-15 exposure on real sample files during the decoder pilot.

## Why a simple dual-pol cut is not safe

[NWS Jackson's dual-pol applications](https://www.weather.gov/jan/dualpolupgrade-applications) explicitly combines products to interpret echoes. Its hail example has CC below 0.9 and as low as **0.7**. Its melting-snow guidance spans **0.88–0.985**. Tornado debris also has low CC. A blanket `CC < 0.8`, `0.9`, or `0.97` deletion can remove important signals. A high-reflectivity exemption alone does not prove safety for all cases.

ZDR near zero occurs in hail as well as clutter; unusual ZDR is not independently a discard condition. Texture can identify variability, but mixed-phase weather, hail edges and genuine narrow features also vary. Low-SNR and distant samples make dual-pol interpretation less reliable. Missing or invalid CC must never mean zero CC.

[Py-ART's source](https://github.com/ARM-DOE/pyart/blob/52645096598bd12d7b3b5500f4d1014fb68db5a3/pyart/filters/gatefilter.py) provides useful reference implementations, not a display standard:

- `moment_based_gate_filter`: `min_ncp=0.5`, `min_rhv=None`, reflectivity interval −20 to 100 dBZ. CC filtering is disabled by default; missing fields skip their corresponding tests.
- `moment_and_texture_based_gate_filter`: 7-gate along-ray window; maxima PHI texture 20, RHO texture 0.3, ZDR texture 2.85, REF texture 8; minimum RHO 0.6. It copies the radar object and computes up to four texture fields.
- These are algorithm/correction gate filters. Their numbers are **not validated universal NEXRAD reflectivity-display defaults**. Port neither all defaults nor the object-copying architecture into a tile request.

A future optional cleaned profile can compare low-CC-plus-texture classification against labeled cases. Start with diagnostics and record error rates; do not pick a deletion threshold from this research alone. This ticket deliberately resolves the safe default rather than inventing a trained classifier.

## What comparable apps document

### Supercell Wx

Inspected the released [Level II product view](https://github.com/dpaulat/supercell-wx/blob/v0.6.2-release/scwx-qt/source/scwx/qt/view/level2_product_view.cpp). Reflectivity selects `MomentRef`; correlation coefficient is a separate `MomentRho` product. The inspected REF rendering path does not fetch CC to cross-mask reflectivity.

Its encoded-value gate condition uses `max(2, snr_threshold_raw())` and preserves range-folded codes separately. Report that as source behavior, **not** as a 2 dBZ cutoff or an independently measured gate-SNR test. Follow NOAA semantics rather than copying that comparison blindly.

The [product settings](https://github.com/dpaulat/supercell-wx/blob/v0.6.2-release/scwx-qt/source/scwx/qt/settings/product_settings.cpp) disable optional palette thresholding by default. The [default REF palette](https://github.com/dpaulat/supercell-wx/blob/v0.6.2-release/scwx-qt/res/palettes/wct/DR.pal) includes negative values down to −20 dBZ. The [toolbox docs](https://supercell-wx.readthedocs.io/en/stable/user-guide/radar-toolbox.html) describe REF as raw radar data and thresholding as optional. “Raw” here means supplied moment data, not unprocessed receiver samples.

### RadarScope

The official [Color Palettes guide](https://radarscope.zendesk.com/hc/en-us/articles/4411527535634-Color-Palettes) documents Standard (Low-Filter) as the default and Expert Mode as including the low end. This is **reflectivity palette filtering**, not evidence of a CC mask; the text gives no numeric cutoff. The complete paragraph is also available in the [official article API](https://radarscope.zendesk.com/api/v2/help_center/en-us/articles/4411527535634.json), which avoids incomplete page extraction.

The official [Dual-Polarization Products guide](https://radarscope.zendesk.com/hc/en-us/articles/4411535488018-Dual-Polarization-Products) describes typical clutter CC below 0.8, hail/melting snow at 0.8–0.97, and decreasing reliability with range. Its **0.9 CC cutoff is for KDP calculation, not reflectivity**. RadarScope is closed source; these docs cannot prove that no other proprietary processing exists. Do not advertise either app as precedent for a universal low-CC deletion.

## CPU result and request budget

A throwaway C probe ran in the Hermes x86_64 pod with `cc -O2 -std=gnu11 -Wall -Wextra -Werror`. It used **synthetic**, already decoded inputs, not a NOAA scan:

- Shape: 720 radials × 1,840 gates = 1,324,800 gates.
- Input bytes: `REF[i] = i % 256`; input floats: `RHO[i] = (i % 101) * 0.01f`.
- Per gate: write an invalid-REF byte (`REF < 2`), write a valid low-CC-candidate byte (`!invalid && finite(RHO) && RHO < 0.8`), accumulate a checksum.
- 200 repetitions, mutating `REF[k] = 2` after each pass; monotonic wall clock.
- Actual first-run output: `mean_ms=6.516 checksum=212904600 sentinel_assertions=5 passed`. A verification rerun returned `mean_ms=3.021` with the same checksum and assertions; scheduling/cache effects make this unsuitable as a stable latency estimate.
- The five assertions checked codes 0, 1, 2, 100 and 255 against expected invalid flags 1, 1, 0, 0 and 0.

This establishes that a fused validity/CC diagnostic pass is inexpensive in this environment. **It does not benchmark download, bzip2, Rust decoder, moment alignment, gridding, PNG encoding, texture, or end-to-end request latency.** No production CPU limit was measured, and this is not a capacity claim.

| Work | Request-path judgment |
| --- | --- |
| Validity, geometry bounds, CC/ZDR scalar tests | O(gates); suitable on one bounded cold-cache worker. Scientific safety, not arithmetic, limits threshold use. |
| Beam-height/range lookup | Precompute per geometry; reuse across radials where appropriate. |
| Seven-gate running texture | Bounded O(gates × window), reducible to O(gates) with running moments; feasible candidate for a worker, but unbenchmarked here and off by default. |
| Multi-field texture, neighborhood/despeckle/classifier | Extra arrays and calibration; not justified on every HTTP/tile request. |
| Static clutter/terrain maps | Cache by station/version if adopted; map membership alone must not delete echoes. |

Proposed implementation gate: measure QC alone at p95 ≤25 ms per lowest-tilt sweep on the target worker under its configured CPU limit. This is a **proposed acceptance target**, not an observed p95. Benchmark cold decode-to-publication separately and coalesce identical requests. QC profiles need bounded memory/concurrency and cannot borrow the API's manifest latency budget.

## Fit to radar-ng

Read `AGENTS.md`, `.claude/commands/rust-dev.md`, `.claude/commands/add-layer.md`, and `docs/reference/north-star.md` at the baseline above. North-star steps 1–4 require QC, beam awareness, range masking, provenance, and MRMS fallback. They do not authorize suppressing severe-weather signatures to make the image cleaner.

- **Server:** apply QC after polar decoding, before gridding. `rust/crates/radar-tiler/src/render.rs` currently accepts a numeric `Grid` with NaN = missing; it is not a polar dual-pol classifier. Do not move scientific QC into a palette or repeat it per tile. Preserve missing coverage through the renderer's sampling path.
- **Publication:** preserve the existing complete-frame/immutable identity contract. A new mask/profile must change artifact identity. Keep numeric values and quality reasons available for validation.
- **API:** `rust/crates/radar-api/src/manifest.rs` caches a published snapshot, coalesces reads and serves last-known-good on read failure. Keep NOAA downloads, QC and heavy decode out of this manifest handler. Add QC profile, station/scan/tilt, range and quality-availability provenance in the eventual frame contract; this note does not settle the schema owned by other map decisions.
- **Web + app:** consume the same server product and provenance. Label it single-site observed reflectivity, not a guaranteed precipitation map. If an optional cleaned/expert mode is added, ship the control and honest legend on both clients. Clients never fetch NOAA directly.

## Validation before enabling close-up regions

Use the north star's 3–5-site, 30-day shadow pilot. Include clear-air insects/birds, AP, wind farms, light rain, dry snow, melting layer, hail (including very low CC), tornado debris, missing RHO/ZDR, range folding, spot blanking, azimuth gaps and mixed gate geometries. Include reviewed archive events for rare hazards rather than waiting for them during the pilot.

Compare supplied REF, conservative QC, candidate low-CC masks and MRMS without treating MRMS as perfect gate truth. Count masked valid weather, residual clutter, holes, retained core maxima, runtime/memory and scan age; have a meteorological reviewer judge safety-critical frames. Require zero loss of valid REF solely because dual-pol is absent, and no sentinel-to-dBZ conversion. Match web/app no-data and fallback behavior.

**Open risks:** surviving clutter is an intentional limit of the default; decoder support for optional CFP/maps is not verified here; no texture or full-pipeline performance benchmark ran; proprietary app algorithms are unknown; terrain/AP and missing-gate interpolation still need pilot evidence. Additional destructive filtering remains disabled until this evidence supports it. No product code or deployment changed.
