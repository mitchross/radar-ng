# Weather Clear Reconciliation Audit

## Baseline

- Device: Android emulator, 1080 × 2400, 420 dpi
- Jest: 6 suites, 26 tests passing
- Lint: passing
- TypeScript: failing at `StatWidgets.tsx:73` because `{ perspective: { px: number } }` is not a valid React Native transform
- Reference: `Weather app redesign/Weather Clear.dc.html`
- Historical evidence: `Weather app redesign/uploads/*` and `frontend/screenshots/*` are matching copies of the old dark UI and are not target captures

## Screen findings

| Screen | Structure | Type/spacing | Color/surfaces | Interaction/state | Accessibility |
| --- | --- | --- | --- | --- | --- |
| Home | Current runtime broadly follows the light redesign, but relaunch can restore a mid-page scroll position and obscure the reference hero during capture | System serif/sans fallbacks differ from Newsreader/Spline Sans | Warm light palette exists but is hard-coded | Live data and scrolling work; stale/error distinctions require audit | Location and card summaries need explicit labels |
| Nowcast | Must be checked against the HTML Simple and Advanced states | Static screen styles can drift independently | Hard-coded light-only surfaces | Missing minutely data is currently presented as loading or dry | Chart needs a textual summary |
| Radar | Full-screen structure is retained | Chrome sizing differs from the 44-point target | Map chrome assumes dark glass | Map data and gesture behavior must be preserved | Several icon-only controls require labels and state |
| Alerts | Empty-state structure exists | Typography uses platform fallbacks | Static light-only card surfaces | Query failure can read as “All clear” | Severity text exists; refresh target needs audit |
| Settings | Functional groups exceed the static reference and must remain | Large single file has repeated row styles | Static light-only surfaces | Appearance preference is absent and map style must remain independent | Inputs, sliders, and icon controls need complete labels |

## Root causes

1. Screen colors are static module constants, so native system appearance cannot propagate.
2. Native font fallbacks do not match the Newsreader/Spline Sans reference.
3. Repeated screen chrome has drifted because it is implemented independently.
4. Several data screens collapse unavailable data into loading or empty states.
5. Checked-in screenshots were copied from the design inputs, so they cannot validate the current native build.

## Evidence

- `/tmp/weather-clear-audit/reference-home.png` — locally rendered Weather Clear HTML target
- `/tmp/weather-clear-audit/native-home.png` — Android runtime baseline
- `/tmp/weather-clear-audit/home-side-by-side.png` — qualitative target/runtime comparison
- `/tmp/radar-ng-home.xml` — baseline Android accessibility hierarchy

## Verification status

This document records the pre-change state. Final results, resolved findings, dark-theme validation, Android build evidence, and the iOS simulator limitation will be appended after implementation.
