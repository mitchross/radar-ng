# radar-ng — App & Page Design Brief

Hand-off doc for design work (Claude Design / Figma / etc). Focuses on what each screen does for the user, not implementation. Code lives in `frontend/src/app/(tabs)/` and `frontend/src/screens/`.

## What it is
A self-hosted personal weather app. The user runs their own stack (MRMS radar, HRRR forecast, Open-Meteo, NWS alerts, Protomaps basemap) on a home server, and this Expo/React Native app is the front end. It's *their* weather, no ads, no telemetry, no third-party service degrading silently.

**Design direction:** bold, expressive, personality-driven — closer to CARROT Weather than to a sterile dark radar-centric tool. Cards with strong color, gradients tied to current condition, big confident type, accent purple `#8B7CFF`, alert red `#FF3B4A`, sun yellow, rain blues.

## Who it's for
- **The home-lab weather nerd** — owns a self-hosted stack, wants to see what their server is doing and tune palettes/opacity. Lives in Settings as much as Radar.
- **The "is it about to rain on me" person** — opens the app, glances at hero temp + nowcast banner + minute-precision rain chart, closes it.
- **The storm watcher** — pulls up Radar, scrubs the timeline back through the last hour and forward into the HRRR forecast, taps to inspect a cell, watches lightning + storm-cell dots.
- **The alert-driven user** — gets a tornado/severe storm warning, taps to read the full NWS bulletin and see the polygon on the map.

## Top user cases
1. **Glance check** — "what's it like outside *right now*". Open app → land on Home → done in 3 seconds.
2. **Will it rain in the next hour?** — Home → tap nowcast banner → Nowcast screen with the 60-min intensity chart.
3. **Watch a storm move** — bottom tab Radar → scrub timeline → toggle layer (radar / temp / wind / CAPE / precip-type / cloud) → long-press to inspect a point.
4. **Read an active warning** — bottom tab Alerts (red badge with count) → tap card → full description + expiry.
5. **Tune the stack** — Settings → check stack health, edit server URL, swap radar palette, change opacity, set playback FPS, see Docker container status.
6. **Refresh anywhere** — pull-to-refresh on every scrollable screen; on Radar, a refresh button on the right rail snaps to "Now" and pauses playback.

## Pages

### 1. Home (`(tabs)/index.tsx`) — the daily glance
**Purpose:** answer "what's it like and what's coming" in one scroll.

**Background:** full-screen LinearGradient that changes with current condition (clearDay/clearNight/cloudy/rain/storm/snow/fog). The whole screen *feels* like the weather.

**Stacked sections (top→bottom):**
- **Top bar** — location pill (`Grand Rapids, MI`) with purple accent dot, "UPDATED 11:47 PM" mono caption, gear button → Settings.
- **Hero** — huge thin 112pt temperature, degree symbol offset, condition word ("Sunny", "Thunderstorms"), "Feels like 72° · H 78° L 61°" line, large WeatherIcon (sun/cloud/rain/snow/storm — custom-drawn shapes, not emoji) floating right.
- **Nowcast banner** (conditional) — translucent blue card, raindrop icon, "Heavy rain starts in 15 min · Lasts ~40 min · 0.32" total". Tap → Nowcast screen.
- **Active alert banner** (conditional) — translucent red card, severity dot, event title + expiry. Tap → alert detail.
- **HOURLY · NEXT 24** — horizontal scroll of 24 cells, each: time / icon / temp / precip%. The "NOW" cell is purple-tinted.
- **PRECIPITATION · 24H** — bar chart, 24 bars, total inches in the section header.
- **7 DAY FORECAST** — list rows: day / icon / precip% / hi-lo gradient bar (cold→sun→hot) with a white dot showing today's current temp / lo / hi.
- **Radar mini-map** — small live MapLibre tile with the user's location and a teaser headline. Tap → Radar tab.
- **Stat grid** (2 cols × 3 rows) — UV / Wind / Humidity / Visibility / Pressure / Dewpoint. Each card: label / big value / unit / sub-label / accent-colored progress bar.
- **SUN & DAYLIGHT** — sunrise / daylight duration / sunset row + horizontal day-progress track with a sun dot.

### 2. Nowcast (`screens/NowcastScreen.tsx`) — the next 60 minutes
**Purpose:** minute-by-minute precip outlook, hyper-local.

- **Header** — back chevron + "HYPER-LOCAL NOWCAST" / location.
- **Hero verdict** — huge thin type, one of: "No rain expected in the next hour" / "Raining now." / "Rain starts in 12 minutes". Sub-line: "Expected to last ~40 min · peaks at +18m".
- **Big chart card** — INTENSITY MM/HR / NEXT 60 MIN. 60 vertical bars, color graded green→blue→purple→red by intensity. Time axis: NOW · +15 · +30 · +45 · +60. Color scale legend underneath: LIGHT ←gradient→ INTENSE.
- **KEY MOMENTS** (2×2 grid) — STARTS / PEAK / ENDS / TOTAL cards, each with an icon tinted by accent color.
- **FORECAST MODEL** card — Model: HRRR + MRMS blend / Resolution: 3 km · 15 min / Confidence: progress bar + % / Last update: N min ago.
- **About this forecast** — small explainer note.

### 3. Radar (`(tabs)/radar.tsx`) — the map
**Purpose:** real-time weather map. Full-bleed, no chrome. Hides the bottom tab bar.

**Layers (right rail picker):**
- Reflectivity (MRMS observed) — main view.
- Reflectivity (HRRR forecast) — switches the timeline emphasis.
- Temperature — colored gradient.
- Wind — animated Skia particle overlay seeded around the camera, dark outline pass.
- CAPE (instability).
- Precip type (rain/snow/sleet/freezing-rain).
- Precip accumulation.
- Cloud cover.

**Always-on overlays:**
- Active NWS alert polygons (severity-tinted).
- Tropical storm cones (when relevant).
- User location marker — pill that shows the *live value of the active layer* under the user (e.g., "62°F" on temperature, "32 dBZ" on radar).

**Optional overlays (extras toggle, off by default):**
- Lightning strike dots.
- Storm cell tracks with vector arrows.

**Chrome:**
- Top-left: small circular "X" close button (goes back to Home).
- Top-center: alert banner if any active warning.
- Top-left below: layer legend card — vertical color scale for whatever layer is active.
- Right rail (5 stacked glass buttons, top→bottom): layer picker / extras toggle (lightning bolt) / map style + projection / inspector crosshair / **refresh** (snaps to Now + pauses + invalidates manifest).
- Bottom: **Timeline bar** — playhead, frame label ("Reflectivity / Sunday, April 19 2026 · 11:24 PM"), play/pause button, scrubbable strip. Past frames (observed MRMS) shown distinctly from forecast frames (nowcast 0–60min, then HRRR 1h–48h).

**Interactions:**
- Long-press anywhere → drops an Eyedropper pin showing precise lat/lon + layer value at that point.
- Map style picker (modal) — basemap theme (light / dark / sat) + projection (web mercator / globe).
- Palette picker (in Settings) — classic / muted / vivid radar color ramps, server-baked.

### 4. Alerts (`(tabs)/alerts.tsx`) — active NWS warnings
**Purpose:** list-of-warnings view, urgent feel.

- **Background:** stormy gradient regardless of weather (this tab is always serious).
- **Header** — "ACTIVE ALERTS" kicker, "3 in your area" or "All clear" title, refresh button (↻).
- **Cards** — each alert: severity-colored left stripe, severity pill (Extreme red / Severe orange / Moderate yellow / Minor blue), urgency caption, big event title ("Severe Thunderstorm Warning"), 3-line headline, area description, "Until Apr 30, 11:45 PM" expiry. Tap → detail modal.
- **Empty state** — green dot in soft halo, "No active alerts", small explainer.
- **Pull to refresh.**

### 5. Alert Detail (`alert/[id].tsx`) — the bulletin
**Purpose:** read the full NWS message.

Currently minimal — needs design love. Should show:
- Severity-colored hero band with event name.
- Area description, sender (NWS office), effective / onset / expires times.
- Full description (long-form NWS text).
- "What to do" instruction block (the CAP `instruction` field).
- Mini-map showing the alert polygon (link out to Radar tab with that polygon centered).

### 6. Settings (`(tabs)/settings.tsx`) — the stack control panel
**Purpose:** this is half the app for a self-hosted user.

- **Title** — "Settings".
- **Hero card** — "SELF-HOSTED STACK" / "radar-ng" / hostname / status dot (ONLINE green pulsing / ERROR red). Three stat columns: UPTIME / TILES/DAY / CACHE.
- **Stack URL** — text input + Save button. Single source of truth, all derived endpoints update.
- **Data Sources** card — collapsible rows. Each: emoji icon, name (Radar tiles MRMS, Forecast HRRR+Open-Meteo, Base map, Satellite GOES, Alerts NWS CAP), endpoint URL (mono font, truncated), status pill (OK / STALE / ERROR / OFF). Tap → expanded editor with URL template / auth / timeout / retries.
- **Docker Stack** card — list of running containers (tile-server, ingest-mrms, ingest-hrrr, nowcast-pysteps, open-meteo, basemap, caddy). Each: status dot / name / image / ports. "Pull & Restart" + "View Logs" big buttons below.
- **Tile Cache** — storage used / 8 GB total with progress bar + "Clear" purge action.
- **Network** — refresh interval picker (30s / 1min / 2min / 5min), "Use cellular for tiles" toggle.
- **Preferences** — Dark basemap toggle, Temperature segmented (°F / °C), Radar palette selector (Classic / Muted / Vivid swatches), Radar opacity slider, Playback speed slider (1–15 FPS).
- **About** — version / server / stack status.
- **Footer** — "radar-ng — Cumulus UI / Self-hosted: MRMS · HRRR · Open-Meteo · Protomaps · NWS alerts".

## Cross-cutting design system
- **Color tokens** (in `frontend/src/lib/cumulusTheme.ts`): `cumulus.ink` (white), `inkDim`, `inkMuted`, `inkFaint`, `inkLine`, `card`, `cardLine`, `cardStrong`, `accent` (purple `#8B7CFF`), `accentSoft`, `accentBorder`, `alert` (red `#FF3B4A`), `ok` (green), `sun` (yellow), `hot`, `cold`, `rain` (blue `#4FB8FF`), `rainHeavy`.
- **Condition gradients** — full-screen backgrounds keyed to weather: clearDay / clearNight / cloudy / rain / storm / snow / fog.
- **Section headers** — uppercase 11pt, letter-spaced 1.6, mono-feel, optional right-side caption.
- **Cards** — translucent `rgba` over the gradient, 16–18 radius, 1px hairline border in `cardLine`. Never opaque solid panels.
- **Mono font** — SF Mono for numbers, timestamps, endpoints, kickers — gives it that "instrument panel" feel.
- **Bottom tab bar** — floating dark-translucent pill, 5 custom-drawn icons (home/nowcast bars/radar dish/alert triangle/gear). Active tab gets purple-soft pill behind icon. Alerts tab shows red badge with count.

## Priority order for design handoff
1. **Home screen** — hero treatment is the signature shot. Get the gradient + huge temp + condition icon right.
2. **Radar timeline bar** — the playhead/scrubber/play button is the most-touched control in the app.
3. **Nowcast chart** — the gradient bar chart is the second signature shot.
4. **Alert card** — severity stripe + pill is a recurring pattern (also used in Home banner and Radar top banner).
5. **Settings hero + Data Sources rows** — sets the tone for "this is YOUR stack" identity.
6. **Empty states** — Alerts "All clear", Nowcast "No rain expected", Settings offline.
7. **Alert Detail screen** — biggest design gap right now (current impl is plain text on dark). Needs the same polish as Home.
