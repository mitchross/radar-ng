# Handoff: Radar-NG (iOS Weather App)

## Overview
Radar-NG is a Carrot-faithful, dark-mode iOS weather app prototype centered on
high-resolution radar (MRMS/HRRR-style) and minute-by-minute precipitation
nowcasts. It is a 4-tab native-feel app (Home · Radar · Alerts · Settings)
designed for iPhone, presented inside an iOS device frame.

## About the Design Files
The files in this bundle are **design references created in HTML/JSX** — a
clickable prototype showing intended look and behavior, not production code to
copy directly. The task is to **recreate these designs in the target codebase's
existing environment** (SwiftUI for native iOS, React Native, or whatever
matches the team's stack) using its established patterns and component library.
If no environment exists yet, SwiftUI is the natural target.

The prototype uses inline React + Babel for fast iteration. Do not ship that
runtime — port the components into the target stack.

## Fidelity
**High-fidelity (hifi).** Pixel-perfect mockups with final colors, typography,
spacing, gradients, motion, and interactions. The developer should match the
visual design closely; the design tokens at `src/design.js` are the source of
truth for colors, fonts, and condition gradients.

## Screens / Views

### 1. Home (`src/home.jsx`)
- **Purpose:** At-a-glance current conditions + the next 24 hours and 7 days.
- **Layout (top → bottom, single scroll):**
  1. **Hero block** — location name (SF Pro Display, 17 medium), big temperature
     (96–120pt thin), condition phrase, hi/lo line. Centered, large vertical
     padding.
  2. **Nowcast banner** — pill-shaped card, full width minus 16px gutter,
     showing minute-by-minute precip text + a tiny inline sparkline (next 60
     min). Tappable → opens nowcast detail.
  3. **24h hourly strip** — horizontal scroll, each cell ~56×120: hour label,
     precip-prob bar, weather icon, temperature.
  4. **7-day forecast** — vertical list, each row: day · icon · low/high range
     bar (gradient cold→hot) with a dot marking today's temp · numeric lo/hi.
  5. **Data cards grid** — 2-col grid of square-ish cards: AQI, UV, Wind,
     Humidity, Pressure, Visibility, Sunrise/Sunset, Dew point. Each card =
     icon top-left, label, big value, micro-context line.
- **Background:** condition-driven gradient (see `conditionBG` in
  `src/design.js`). Storm = violet radial; clear day = blue radial; etc.
- **Pull-to-refresh:** `src/pull-refresh.jsx` — drag down past ~60px reveals an
  animated radar-sweep spinner; releasing triggers a 1.2s refresh.

### 2. Radar (`src/radar.jsx`)
- **Purpose:** The flagship screen. Full-bleed animated radar with timeline
  scrubber and layer controls.
- **Layout:**
  - **Full-bleed map** behind everything — dark basemap (rgba black 0.85
    overlay on a procedural city/coastline SVG). Animated radar reflectivity
    blobs drift across (see `radar.jsx` `RadarCanvas`).
  - **Top bar:** back chevron · location · "now / +1h" segmented control (right).
  - **Glass layer panel (right):** floating card, ~60×260, vertical icon stack
    for layers: Reflectivity, Velocity, Precip-type, Lightning, Satellite,
    Alerts. Active = filled accent (`#8B7CFF`).
  - **Legend (left):** tiny vertical dBZ scale, 5–75 dBZ, using the `dbz`
    palette in `design.js`.
  - **Bottom forecast pill:** floating card with current condition + temp +
    "Light rain in 12 min" copy.
  - **Timeline scrubber** above the tab bar: thumb on a translucent track
    spanning -2h → now → +1h, with tick marks every 15 min and a Play button
    at left.
- **Interactions:** scrubber drags update radar frame; play loops -2h→+1h at
  ~1.2s/loop; layer taps toggle overlays.

### 3. Alerts (`src/alerts.jsx`)
- **Purpose:** Active NWS-style alerts for the saved locations.
- **Layout:** Vertical list of alert cards. Severe = red (`#FF3B4A`) left rail
  + bold headline; advisory = amber; statement = neutral. Each card: tag chip,
  headline, expires-in, location, 2-line summary, "View polygon" link.
- **Empty state:** centered shield-check icon, "No active alerts."

### 4. Settings (`src/settings.jsx`)
- **Purpose:** Units, notifications, data sources, about.
- **Layout:** iOS-grouped table style — translucent rounded sections on the
  condition gradient. Sections:
  - Units (°F/°C, mph/kph, in/mm)
  - Notifications (Severe alerts, Rain in 30 min, Daily summary — toggles)
  - Data sources (MRMS · HRRR · NWS — read-only chips with status dots)
  - About (version, "Radar-NG is open source", links).

## Interactions & Behavior

- **Tab navigation:** bottom tab bar, 4 items, blur background, active tab in
  accent. Implemented in `src/app.jsx`.
- **Pull-to-refresh** on Home — see above.
- **Radar scrubber** — `pointerdown`/`pointermove` on the track; thumb snaps to
  15-min ticks on release.
- **Layer toggles** — instant; no confirm.
- **Tweaks panel** (dev only, behind `__edit_mode_available`): lets you change
  condition (clear/cloudy/rain/storm/snow/fog), time of day, and temperature
  unit. Not part of the user-facing app.

## State Management
The prototype keeps everything in React `useState` at the `App` level. For
production:
- **Current location + saved locations:** persisted store.
- **Tab state:** ephemeral.
- **Radar frame index, play/pause, active layers:** ephemeral.
- **Units, notification prefs:** persisted.
- **Live data:** poll MRMS tiles every 2 min, HRRR every 15 min, NWS alerts
  every 5 min.

## Design Tokens
Authoritative source: `src/design.js`. Summary:

**Accent**
- `accent` `#8B7CFF` · `accentBright` `#A594FF` · `accentDim` `#5B4FD6`

**Ink (text on dark)**
- `ink` `#FFFFFF`
- `inkDim` `rgba(255,255,255,0.72)`
- `inkMuted` `rgba(255,255,255,0.48)`
- `inkFaint` `rgba(255,255,255,0.28)`
- `inkLine` `rgba(255,255,255,0.10)`

**Card surfaces**
- `card` `rgba(255,255,255,0.06)`
- `cardStrong` `rgba(255,255,255,0.10)`
- `cardLine` `rgba(255,255,255,0.08)`

**Data colors**
- rain `#4FB8FF` · rainHeavy `#1E7FFF` · snow `#C7E6FF`
- sun `#FFC14D` · temp `#FF6E3A` · cold `#5BD4FF` · hot `#FF4D6D`
- alert `#FF3B4A` · ok `#4ADE80`

**Radar dBZ scale** (low → high):
`5:#7ae5a8 · 15:#3bc77a · 25:#f5d042 · 35:#ff9f2e · 45:#ff4040 · 55:#d02058 · 65:#b24bff · 75:#ffffff`

**Condition background gradients** — see `conditionBG` in `design.js`. Apply
to the screen root; transition with a 600ms ease when condition changes.

**Typography**
- Display: SF Pro Display (hero temp, headlines)
- UI: SF Pro Rounded (chrome, labels, buttons)
- Mono: SF Mono (data readouts, dBZ values, timestamps)

**Spacing scale:** 4 / 8 / 12 / 16 / 20 / 24 / 32 / 48
**Radius:** cards 16, pills 999, inner chips 10
**Shadows:** subtle — most depth comes from layered translucency, not blur

## Assets
- All icons are inline SVG in `src/icons.jsx` (sun, cloud, rain, snow, storm,
  wind, droplet, eye, sunrise, sunset, gauge, shield, layers, play, etc.).
- The radar basemap is procedurally drawn (no raster art).
- iOS device frame: `ios-frame.jsx` (status bar, home indicator, dynamic island).

## Files (in this bundle)
- `Radar-NG.html` — entry point; loads React + all JSX modules.
- `ios-frame.jsx` — device chrome.
- `src/design.js` — design tokens (colors, gradients, fonts).
- `src/icons.jsx` — SVG icon set.
- `src/pull-refresh.jsx` — pull-to-refresh primitive.
- `src/home.jsx` — Home tab.
- `src/radar.jsx` — Radar tab (the headline screen).
- `src/nowcast.jsx` — minute-by-minute nowcast detail.
- `src/alerts.jsx` — Alerts tab.
- `src/settings.jsx` — Settings tab.
- `src/app.jsx` — App shell, tab nav, tweaks panel.

## Implementation Notes for the Developer
- **Native iOS (recommended):** SwiftUI with a `TabView`, `MapKit` or
  `MapLibre` for the radar basemap, `Canvas` or `Metal` for radar tile
  rendering. Use Apple's SF Pro family directly.
- **React Native:** `@react-navigation` bottom tabs; `react-native-maps` +
  custom tile overlay; `react-native-svg` for icons; `Reanimated` for the
  scrubber and pull-to-refresh.
- The prototype animates everything via CSS transitions and `requestAnimationFrame`
  in JSX; port the timing values (durations, easings) literally.
- Condition gradients should crossfade, not snap, when conditions change.
- Keep the radar map dark and low-contrast — the radar reflectivity is the
  hero, the basemap is just context.
