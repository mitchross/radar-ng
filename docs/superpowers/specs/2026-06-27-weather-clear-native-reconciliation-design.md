# Weather Clear Native Reconciliation

**Date:** 2026-06-27
**Status:** Approved for implementation planning

## Objective

Bring the Expo React Native application into close visual and behavioral alignment with the completed Weather Clear design while preserving the production weather, radar, location, alert, and settings behavior.

The result must be a native phone application for iOS and Android. Web support is out of scope.

## Design Authority

The reference artifacts have this precedence:

1. `Weather app redesign/Weather Clear.dc.html` defines intended layout, hierarchy, interactions, and component states.
2. `Weather app redesign/uploads/01-home.png` through `05-settings.png` define the canonical light-theme appearance at 1080 × 2400.
3. `Weather app redesign/Weather Redesign.dc.html` is an exploratory concept board. Its Clear direction may clarify intent but does not override the completed Weather Clear artifact.
4. Existing application behavior is authoritative where the static design does not specify dynamic or failure behavior.

Reference copy and values such as temperature, location, timestamps, alerts, and forecast rows are examples. The application continues to render live data.

## Scope

The reconciliation covers:

- Home in Simple and Advanced modes
- Nowcast in Simple and Advanced modes
- Full-screen Radar and its controls
- Alerts empty, loading, error, and active-alert states
- Alert detail
- Settings, including interactive controls
- Shared tab navigation, location selection, safe areas, loading states, empty states, and errors
- Light, Dark, and System appearance preferences
- Android and iOS phone layouts
- Accessibility and reduced-motion behavior

Tablet-specific redesign, backend changes, new weather features, and web support are out of scope.

## Architecture

### Design system

The current `cumulus` constant is replaced or wrapped by semantic theme tokens rather than expanding screen-level color conditionals. Tokens cover:

- canvas, elevated surface, strong surface, separators, and overlays
- primary, secondary, muted, and faint text
- accent, accent-soft, success, warning, destructive, precipitation, temperature, and radar colors
- typography roles for editorial display, UI text, labels, and monospaced diagnostics
- spacing, corner radii, borders, shadows, icon sizes, and control heights

Light tokens are derived directly from Weather Clear. Dark tokens preserve its warm editorial identity with dark neutral surfaces, readable contrast, and the same accent hierarchy. System mode resolves through the native color scheme. The explicit user preference remains authoritative across app launches.

Shared primitives should be introduced only where repeated behavior or styling justifies them. Likely boundaries are screen header, segmented control, section label, card/surface, setting row, empty state, and tab item. Existing domain components remain intact when their responsibilities are already clear.

### Data and state

Existing React Query hooks, Zustand actions, MMKV persistence, and API contracts remain the data layer. Screens transform live data into the reference hierarchy; they must not hard-code the design’s sample weather values.

The pre-existing uncommitted default-location changes in `frontend/src/stores/useWeatherStore.ts` belong to the user and must be preserved. They may be integrated only if a failing test demonstrates a direct requirement.

Appearance preference is an application preference, distinct from the existing radar map-style preference. Light/Dark/System must not silently alter the selected radar basemap.

### Navigation

The five-tab information architecture remains Home, Nowcast, Radar, Alerts, and Settings. The tab bar matches the reference on normal tabs and remains hidden for the full-bleed radar experience. Native back behavior, deep-linked alert detail, and accessibility selection state remain supported.

## Screen Behavior

### Home

Home follows the reference hierarchy: location and mode controls, current condition and temperature, hourly forecast, precipitation summary, seven-day forecast, and Advanced-mode condition metrics. Content uses live values and supports refresh, loading, partial-data, and error states without collapsing the layout.

### Nowcast

Nowcast prioritizes the next-hour precipitation statement and intensity graph, followed by key moments. Advanced mode exposes model details. Empty precipitation data must distinguish “no rain expected” from “forecast unavailable.”

### Radar

Radar remains a full-screen native MapLibre view. Reference styling applies to overlays, legend, playback timeline, close/navigation affordance, and floating controls. Map gestures, layer selection, frame playback, weather overlays, and location markers remain functional.

### Alerts and alert detail

Alerts implements visually intentional loading, failure, no-alert, and active-alert states. Active items communicate severity without relying on color alone. Alert detail preserves the full NWS content, timing, affected area, instructions, and navigation.

### Settings

Settings uses the reference grouping and row hierarchy while retaining every functional setting supported by the application. Appearance adds Light, Dark, and System. Radar map theme remains a separate control. Diagnostics and server status remain readable but visually subordinate to user-facing preferences.

## Responsive and Platform Rules

- Match the 1080 × 2400 Android captures at their logical-density layout, not by using physical pixel constants.
- Respect native safe-area insets, display cutouts, dynamic text, and bottom gesture/navigation regions.
- Use platform-neutral React Native layout except where native conventions require a small divergence.
- Test narrow and representative phone widths so controls do not clip or depend on one screenshot size.
- Prefer bundled or platform-safe typography with deterministic fallbacks on both operating systems.

## Accessibility

- Interactive controls have roles, labels, state, and at least 44 × 44 logical-point targets.
- Text and essential icons meet readable contrast in both themes.
- Alert severity, selected state, graph meaning, and status do not depend on color alone.
- Dynamic type is supported without hiding critical information.
- Decorative animation respects reduced-motion preferences.

## Error and Transitional States

Every data-backed screen defines:

- initial loading without misleading sample values
- pull-to-refresh or explicit refresh feedback where supported
- stale-but-usable data presentation
- network/server failure with a useful retry path
- partial data without crashing the entire screen
- true empty state distinct from unavailable data

Radar tile and map failures must leave navigation and recovery controls usable.

## Audit and Implementation Method

1. Capture a clean Android emulator baseline for every tab and relevant state.
2. Compare the baseline against the canonical screenshots and interactive HTML.
3. Inventory mismatches by structure, typography, spacing, color, component styling, interaction, state handling, accessibility, and platform behavior.
4. Fix shared tokens and primitives before screen-specific differences.
5. Reconcile one screen or interaction slice at a time using a failing behavioral or regression test before production changes.
6. Recapture the same emulator states and perform side-by-side visual review.
7. Run the complete automated verification suite and native smoke checks.

Visual matching is judged by hierarchy and logical layout first, then typography, spacing, color, radii, borders, shadows, and icon treatment. Dynamic live content is not expected to pixel-match sample values.

## Testing Strategy

### Automated

- Unit tests for token selection, appearance persistence, data-to-view formatting, and state distinctions.
- Component tests for mode selection, tab accessibility, settings behavior, loading/error/empty rendering, and alert severity semantics.
- Existing store, API, geocoding, weather-code, tile URL, and map tests remain green.
- TypeScript and lint verification cover both platform branches.

Tests follow red-green-refactor for each behavior change. Purely visual constants that cannot be usefully behavior-tested are validated through deterministic screenshots and code review rather than brittle style-object snapshots.

### Native runtime

Android verification uses the available emulator and ADB for:

- launch and navigation through all five tabs
- Simple/Advanced switching
- appearance switching
- location selection
- alert navigation when data is available or fixture-driven
- radar controls and back navigation
- screenshots at consistent device state and viewport
- runtime log inspection for crashes and warnings

This Linux environment cannot run an iOS simulator. iOS confidence comes from shared-code tests, TypeScript, lint, safe-area and platform-branch review, and avoidance of Android-only assumptions. Final iOS visual acceptance requires a macOS simulator or physical-device pass outside this workspace.

## Acceptance Criteria

- The five primary light-theme screens closely match the canonical Weather Clear hierarchy and visual language.
- Dark mode is a coherent Weather Clear adaptation, and System follows the native preference.
- All existing weather, location, radar, alert, navigation, persistence, and settings behavior remains functional.
- Loading, error, stale, partial, and empty states are distinguishable and recoverable.
- Android emulator smoke flows complete without crashes or new serious warnings.
- Automated tests, type checking, and lint pass.
- No new Android-only implementation breaks the shared iOS code path.
- The user’s pre-existing uncommitted store changes are preserved.

## Deliverables

- Audited and reconciled React Native implementation
- Focused regression and behavior tests
- Before/after Android screenshots or comparison artifacts
- A concise findings summary covering resolved mismatches, remaining platform limitations, and verification evidence
- Mink notes for durable design decisions, verified root causes, and native verification gotchas
