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

This document records the pre-change state. Final results are recorded below.

### Final results (post-implementation)

- **TypeScript**: passing (`tsc --noEmit` clean). The baseline `StatWidgets.tsx:73` invalid-transform failure is resolved.
- **Jest**: 10 suites, 54 tests passing (baseline: 6 suites, 26 tests). Added coverage for the Weather Clear theme, presentation contracts, the store, and location labels.
- **Lint**: passing (`expo lint`).

### Resolved findings

- Screen colors now flow from `weatherClearTheme` / `WeatherClearThemeProvider` rather than static module constants, so native system appearance propagates (root cause 1). Appearance preference is now selectable in Settings, independent of map style.
- Newsreader/Spline Sans reference typography is wired through the theme (root cause 2).
- Shared screen chrome (radar FABs, timeline bar, map style picker) is reconciled to the 44-point target and consolidated (root cause 3).
- Editorial screen headers use the compact `activeLocationName` (city name only) while Settings and accessibility labels retain the full `activeLocationLabel`.
- A default place (Grand Rapids) seeds the store so latitude/longitude are never null on first launch, removing the 0°/blank-data first-run state.

### Known limitation

- iOS simulator validation is unavailable on this Linux host; visual validation was performed on the Android emulator (1080 × 2400, 420 dpi). Dark-theme rendering is covered by `weatherClearTheme.test.ts` and the theme provider.
