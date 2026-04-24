# Handoff: Radar App (self-hosted weather)

## Overview
A self-hosted iOS weather app with Apple-Weather-grade polish and pro-level radar depth. Five primary surfaces in this bundle:

1. **Home** — hero temperature, condition-driven gradient background, 24h hourly strip, precipitation chart, 7-day forecast, nowcast banner, details grid.
2. **Nowcast** — 60-minute precipitation bar chart with confidence band, hyper-local hero ("Rain in 12 min"), intensity legend, key moments grid.
3. **Radar** — Apple-Weather-style full-bleed map. Four layers (Precipitation, Temperature, Air Quality, Wind). Non-linear timeline from −1h to +24h with 1h/12h zoom toggle. Floating layer/legend/forecast panels (vibrant glass).
4. **Settings** — **self-hosted data-source configuration**: per-source endpoint + auth + status, Docker container health, tile cache, network.
5. **Persistent bottom tab bar** with Home · Nowcast · Radar · Alerts · Settings.

---

## About the Design Files
These are **design references** — React + inline Babel prototypes running from a single HTML file. Treat them as visual + interaction specs for the intended product, **not production code to ship**.

Recreate these designs in the target codebase's environment (SwiftUI is the natural fit for iOS; React Native or Flutter also work). Do not port the `window.CUMULUS` global, the iOS-frame scaling wrapper, or the inline `<script type="text/babel">` loader — those are presentation scaffolding.

## Fidelity
**High-fidelity.** Exact colors, sizes, spacing, and interactions. Recreate pixel-perfectly using the target platform's idiomatic components.

---

## Screens

### 1. Home
**Purpose:** At-a-glance conditions for the user's primary location.

**Layout top → bottom:**
- Location row — city name (SF Pro Rounded Semibold 17px) + small pin + "My Location" chip.
- Hero temp — SF Pro Display Thin **~96px**, letter-spacing −4%, white. Below: condition label (SF Pro Rounded Medium 17px, `rgba(255,255,255,0.72)`). Then `H: 72° L: 58°` in mono 13px.
- Nowcast banner — full-width card, 12px radius, `rgba(255,255,255,0.06)` fill. Left icon · "Rain in 12 min" headline · sub-caption · chevron. Tap → Nowcast.
- 24h Hourly strip — horizontal scroll, 44px cells: hour (mono 11px) · condition icon (24×24) · temp (Rounded Semibold 15px).
- 7-Day forecast — 7 rows: day · icon · temp gradient bar (cold `#5BD4FF` → hot `#FF4D6D`) with today's temp as white dot · lo/hi mono 13px.
- Details grid (2×3) — UV, Humidity, Wind, Pressure, Visibility, Dew. 12px radius cards, `rgba(255,255,255,0.06)` fill, label 11px uppercase, value 20px semibold.

**Background:** condition-driven full-screen gradient (see `src/design.js` → `conditionBG`).

### 2. Nowcast
- Header: back chevron, "Nowcast", "Next 60 minutes".
- Hero verdict: "Rain starts in **12 min**" — minutes in Display Semibold 64px violet; supporting text Rounded 20px white.
- Precipitation bar chart: 60 bars (one per minute), height = mm/hr, color per dBZ band. Confidence band (±1σ) as faint violet fill behind bars. X-axis: Now · +15 · +30 · +45 · +60.
- Intensity legend: horizontal gradient strip labeled Light · Mod · Heavy · Extreme.
- Source footer: "MRMS + HRRR blend · updated 2 min ago" (mono 11px).

### 3. Radar (Apple Weather-style)
**Map base** varies by layer:
- Precipitation: off-white land (`#e6efe8`), grey-blue water (`#c6dbe8`), dark labels.
- Temperature: land uses a blue→green→yellow→orange linear gradient (fixed reference to real temp scale), labels inverted.
- Air Quality: green-tinted base with AQ patches as yellow/orange blobs.
- Wind: dark slate-blue base with diagonal streak pattern that shifts with time; light labels.

**Top controls:**
- Left: close (X) circle button.
- Right stack: layers picker · compass/bookmark · list/menu. All 40px white-translucent glass circles with dark icons.

**Layer menu** (top-right, opens below layer button): vibrant glass panel (`rgba(235,245,255,0.88)`, blur 28px saturate 180%), 18px radius, 4 rows with SF-Symbol-style icons + checkmark on selected. Options: Precipitation · Temperature · Air Quality · Wind.

**Left legend** — floating card 80px wide at top-left. Shows the current layer's name + vertical gradient strip with numeric scale ticks (e.g., Temperature: 130 / 90 / 60 / 30 / 0 / −40 °F).

**User location** — white pill on map with reading matching active layer (45° / 45°F / 35 AQI / W 15 MPH). Adjacent "My Location" label. Pulsing halo.

**Map labels** — real city typography with haloed text (paint-order stroke), 9–11px SF Pro, Cleveland/Detroit/Chicago/Indianapolis sorts. State codes (MI, OH, IN) and italic water labels (Lake Erie, Lake Huron).

**Bottom forecast pill:**
- Glass panel, `rgba(255,255,255,0.82)`, blur 32px saturate 180%, radius 28px, heavy drop shadow.
- Row 1: Play/pause button (30×30 dark circle) · "Forecast / Sunday 16:12" text · **1h / 12h** segmented (white on pressed, pill background `rgba(11,18,32,0.08)`).
- Row 2: timeline track (see Interactions).

### 4. Settings — **Self-hosted data sources** (important)
This is the distinguishing feature. All data feeds, including radar tile provider, are user-configurable because the user runs their own backend.

**Sections (scrollable):**

**Data Sources** — list of 5 rows, each with:
- Source name (Radar tiles · Satellite · Forecast model · Basemap · Alerts)
- Status dot (green = live, amber = degraded, red = error, grey = disconnected)
- Current endpoint URL (mono, truncated)
- Latency / last-refresh stat (mono 11px)
- Tap → inline editor expands showing:
  - URL template (e.g., `https://tiles.local/radar/{z}/{x}/{y}.png?t={time}`)
  - Auth method (None · API Key · Basic · Bearer token) — radio
  - Headers (key/value pairs, add/remove)
  - Timeout (slider 2–30s, default 8s)
  - Retry count (slider 0–5)
  - Test connection button → live response code + RTT
  - Save / Revert

**Docker Containers** — health cards, one per container: name (e.g., `rainviewer-tiles`), image tag, ports, uptime, CPU/mem sparkline, restart button. Read from local Docker socket or user-specified API.

**Tile Cache** — bar showing % of max (e.g., 4.2 GB / 8 GB), purge button, auto-evict policy dropdown (LRU / age-based / manual).

**Network** — refresh interval (30s / 1m / 5m / 15m), WiFi-only toggle, VPN requirement toggle, cellular data limit.

**About** — app version, backend version, build hash, support links.

**Visual language:** Dark glass cards on the condition gradient (matches the rest of the app, not a white iOS Settings clone). All mono text for URLs/ports/hashes uses SF Mono.

### 5. Bottom Tab Bar (persistent, hidden on Radar)
- 5 tabs. Dark translucent pill (`rgba(10,10,20,0.55)` + blur 24px) floating 18px above safe area.
- Home · Nowcast · Radar · Alerts (badge count) · Settings (gear icon).
- Active: white icon + white label. Inactive: `rgba(255,255,255,0.55)` + 70% icon opacity.

---

## Interactions & Behavior

### Radar timeline
- **1h mode** — shows −60 to +60 min, 5-minute tick resolution. Labels every 15 min.
- **12h mode** — shows −60 min to +24h, 1-hour tick resolution. Labels every 3h.
- Play/pause advances every **420ms**. Loops within the active zoom window.
- Scrubber drag pauses playback. Release resumes only if it was playing before.
- NOW position marked by a 2px black vertical line + "Now" label.
- Thumb: 16px white circle with 1.5px dark border and a soft drop shadow.

### Layer switching (Radar)
- Tap layer in menu → panel closes, map base + overlay + legend + location-pill reading all swap (200ms cross-fade).
- Overlays:
  - **Precipitation** — animated cell blobs drifting with simulated wind (velocity proportional to minute offset).
  - **Temperature** — large warm/cool patches.
  - **Air Quality** — yellow/orange pollution patches.
  - **Wind** — moving diagonal streak pattern + high-wind patches; speed of streaks maps to hero wind.

### Settings editor
- URL template has syntax highlighting for `{z}/{x}/{y}/{time}` tokens.
- Test connection → immediate UI feedback (spinner → 200 OK 82ms / or red error).
- Status dots update live at the configured refresh interval.

### Home → Nowcast / Radar
- Tap nowcast banner → slide-up to Nowcast screen.
- Tap radar tab → immediate crossfade (radar has its own light-themed background).

---

## State Management
- `currentScreen`: `'home' | 'nowcast' | 'radar' | 'alerts' | 'settings'`
- **Radar:** `frame` (0..FRAMES.length−1), `playing` bool, `layer` ('precipitation' | 'temperature' | 'air' | 'wind'), `zoom` ('1h' | '12h'), `layerOpen` bool.
- **Settings:** `sources` array, `expandedSourceId`, `cache` object, `network` object.
- **Ambient:** `condition`, `timeOfDay`, `accent`.

Weather data is mocked via `buildData(tweaks)` in `src/app.jsx` — replace with your self-hosted API calls.

---

## Design Tokens

See `src/design.js` for the canonical source. Key values:

### Colors
| Token | Value | Usage |
|---|---|---|
| `accent` | `#8B7CFF` | Violet — active states, nowcast, thumb |
| `ink` | `#FFFFFF` | Primary text |
| `inkDim` | `rgba(255,255,255,0.72)` | Secondary |
| `inkMuted` | `rgba(255,255,255,0.48)` | Tertiary |
| `card` | `rgba(255,255,255,0.06)` | Card fill |
| `cold` / `hot` | `#5BD4FF` / `#FF4D6D` | Temp gradient endpoints |
| `ok` | `#4ADE80` | Healthy status, NOW marker |
| `alert` | `#FF3B4A` | Severe warnings, errors |

### Radar / Precipitation dBZ palette
5 `#7ae5a8` · 15 `#3bc77a` · 25 `#f5d042` · 35 `#ff9f2e` · 45 `#ff4040` · 55 `#d02058` · 65 `#b24bff` · 75 `#ffffff`

### Radar layer legends
- **Precipitation:** Light `#7ec4ff` → Moderate `#3b6dff` → Heavy `#7a3bff` → Extreme `#ffd74a` (reversed on the vertical legend).
- **Temperature (°F):** −40 `#1a0f4a` → 0 `#3a3fa8` → 30 `#4fb8ff` → 60 `#7ae55f` → 90 `#ff6e3a` → 130 `#8b1a3a`.
- **Air Quality (AQI):** 0 `#4ADE80` → 100 `#f5d042` → 200 `#f59f3a` → 300 `#d4524e` → 400 `#b03a66` → 500 `#8B1A5B`.
- **Wind (mph):** 0 `#4e78b5` → 25 `#7ea8d9` → 50 `#c5dbff` → 75 `#ffffff`.

### Condition backgrounds
Full gradients in `design.js`. `rain`, `storm`, `clearDay`, `clearNight`, `cloudy`, `snow`, `fog`.

### Typography
- **Display** (hero): `"SF Pro Display", -apple-system, system-ui, sans-serif`
- **UI** (body/labels): `"SF Pro Rounded", "SF Pro Display", -apple-system, system-ui, sans-serif`
- **Mono** (data, URLs, ports): `"SF Mono", "JetBrains Mono", ui-monospace, monospace`

Scale: 96 hero · 28 h1 · 20 h2 · 17 body-large · 15 body · 13 caption · 11 micro uppercase 8% tracking · 13 mono.

### Spacing & shape
- Scale: 4 · 8 · 12 · 16 · 20 · 24 · 32 · 48 · 64 px. Prefer 12/16/20.
- Radius: pills 9999 · cards 12 · hero cards 20 · Radar forecast pill 28 · glass buttons 20 (circle).
- Shadows: glass panels `0 12px 32px rgba(20,30,60,0.16)`; thumb `0 2px 6px rgba(0,0,0,0.2)`.

---

## Assets
- **Icons:** custom SVG in `src/icons.jsx`. On iOS, replace with **SF Symbols** (`sun.max.fill`, `cloud.rain.fill`, `wind`, `aqi.medium`, etc.).
- **Fonts:** SF Pro stack — system default on Apple; fall back to Inter on web.
- **No raster imagery.** All visuals are SVG + CSS.

---

## Files in this bundle
- `Cumulus.html` — entry point. Scales a 402×874 iOS frame to fit viewport.
- `ios-frame.jsx` — iOS device bezel + status bar (presentation only — drop in production).
- `src/design.js` — **canonical design tokens.**
- `src/icons.jsx` — custom SVG icon set. Replace with SF Symbols.
- `src/home.jsx` — Home screen.
- `src/nowcast.jsx` — Nowcast screen.
- `src/radar.jsx` — Radar with Apple-style map, layers, timeline.
- `src/settings.jsx` — Settings with self-hosted data sources + Docker + cache + network.
- `src/app.jsx` — shell, tab bar, state, data-mocking (`buildData`).

Open `Cumulus.html` in any browser to explore the live prototype.

---

## How to use this with Claude Code

1. Download the zip (the card below this message) and unzip into your repo, e.g. `my-weather-app/design/radar_app/`.
2. Run Claude Code from the repo root:
   ```
   claude "Read ./design/radar_app/README.md and implement these designs in our SwiftUI codebase. Use our existing component library and networking layer. Start with the Settings > Data Sources screen since it drives everything else."
   ```
3. Claude Code will read the README, open the HTML prototypes as visual reference, and implement screen by screen using your codebase's conventions.

**Tip:** implement the Settings screen first — the radar/home/nowcast screens all depend on the data-source configuration it defines. Then radar, then home+nowcast.
