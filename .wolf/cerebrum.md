# Cerebrum — radar-ng

## User Preferences

- Tasks are delivered as detailed specs with exact code to implement — follow them precisely without deviation.
- Report format: Status line, then bulleted what was implemented, files created/modified/deleted, test results, any issues.

## Key Learnings

- The existing `src/app/_layout.tsx` was already a custom project layout (not stock Expo template) — it used AnimatedSplashOverlay and AppTabs components. Tasks 6/7 replaced it entirely with the new provider-based layout.
- Tests use `testEnvironment: node` (not jsdom), roots point to `__tests__/` directory with subdirs `lib/` and `stores/`.
- The project already had all required npm packages installed: @tanstack/react-query, expo-location, react-native-gesture-handler, expo-status-bar.
- expo-router uses file-based routing; `(tabs)` is a route group (parentheses = no URL segment).
- Zustand store uses `currentFrameIndex: -1` as sentinel for "not yet initialized".
- MapLibre React Native v10+ uses `mapStyle` prop (not `styleURL`), and `MapViewRef` type (not `MapLibreGL.MapView`).
- RainViewer free tier: max zoom 7, Universal Blue color scheme only, past 2hr radar (~13 frames at 10min intervals), nowcast array may be empty. REPLACED with IEM NEXRAD for free tier.
- IEM (Iowa Environmental Mesonet) NEXRAD tiles: TMS (not XYZ) — requires `tms={true}` on RasterSource. URL pattern: `mesonet.agron.iastate.edu/cache/tile.py/1.0.0/nexrad-n0q-{suffix}/{z}/{x}/{y}.png`. Frames are deterministic (no manifest API call needed) — current is `nexrad-n0q-0`, 5min ago is `nexrad-n0q-m05m`, up to `nexrad-n0q-m50m`. Proper NWS green/yellow/red/magenta colors.
- No backend needed for Phase 1 — IEM NEXRAD (radar tiles), Open-Meteo (forecasts), NWS API (alerts) all free, no auth.
- Expo SDK 55 default template uses `src/app/` (not `app/`) for Expo Router.

## Do-Not-Repeat

- (2026-04-14) Do not assume wolf files exist — check first, create if missing.
- (2026-04-14) In slippy map tile coords, Y increases downward (north = smaller Y). When computing tile range from lat bounds: `tx_min, ty_min = _lat_lon_to_tile(lat_max, lon_min, z)` and `tx_max, ty_max = _lat_lon_to_tile(lat_min, lon_max, z)`. Do NOT swap ty_min/ty_max.
- (2026-04-14) react-native-mmkv v3+ exports `MMKV` as `export type` only — it cannot be instantiated with `new MMKV()`. Use `createMMKV({ id: "..." })` from `react-native-mmkv` instead.
- (2026-04-14) When useManifest returns a union type (RainViewerManifest | SelfHostedManifest), consuming components must use a type guard to access properties specific to one variant (e.g. `manifest.host` only exists on RainViewerManifest).
- (2026-04-14) `tabBarShowIcon` does not exist in expo-router Tabs options. Use `tabBarIcon: () => null` to hide icons instead.
- (2026-04-15) Nullish coalescing `??` mixed with `||` or `&&` requires explicit parentheses: `x ?? (a || b)` not `x ?? a || b`. JavaScript spec/Babel error.
- (2026-04-15) Adding a new expo native module (e.g. expo-linear-gradient) requires full native rebuild via `npx expo run:android`. Dev client must be reinstalled; Expo Go won't include it.
- (2026-04-22) Reanimated `useDerivedValue(() => helperFn(...))` does NOT reliably promote a separately-defined `const helperFn = () => { "worklet"; ... }` to the UI thread. Symptom: the outer worklet runs but the inner helper silently returns an empty value (e.g. empty Skia Path). Always INLINE the worklet body directly inside useDerivedValue (and useFrameCallback). Repeat the body across bins/paths rather than factoring into a shared helper.
- (2026-04-22) For Skia particle overlays on top of colored basemap/heatmap, always draw a dark outline pass (strokeWidth ~2× core, color rgba(10,15,30,0.85)) UNDER the bright core pass. Without the outline, white/light-blue particles disappear against green heatmap tiles.
- (2026-04-22) For camera-relative particle systems: seed particles in a bounded box (±3° lat/lon at zoom 7-8) around camera center and respawn-on-outOfBox each frame. Seeding uniformly across the data's full extent (CONUS ~26°×50°) puts >95% of particles off-screen at typical zoom.
- (2026-04-23) CarPlay code for 3rd-party apps lives in the MAIN iOS app target as a CPTemplateApplicationScene — NOT a separate extension target. `@bacons/apple-targets` handles watch/widget extensions, but not CarPlay scenes; the CarPlay Swift files need to be copied into the main app via a custom config plugin (withDangerousMod + withXcodeProject).
- (2026-04-23) IEM NEXRAD tiles are TMS, not XYZ. For `MKTileOverlay.url(forTilePath:)` you must Y-flip: `tmsY = (1 << path.z) - 1 - path.y`. URL base: `https://mesonet.agron.iastate.edu/cache/tile.py/1.0.0/nexrad-n0q-0/{z}/{x}/{tmsY}.png`.
- (2026-04-23) Apple's `com.apple.developer.carplay-maps` entitlement is NOT granted on dev portal without Apple approval (weather apps never approved). Personal-use workaround: archive with standard signing, then `codesign --force --entitlements patched.plist` with the carplay-maps key present. iOS loads signed entitlements, not portal-provisioned ones, so CarPlay scene connects on device. Only works with paid dev account (free tier strips custom entitlements during install).

## Key Learnings

- react-native-mmkv v3+ uses `createMMKV({ id })` factory (not `new MMKV()`). `MMKV` is only a type export.
- Store tests mock `../../src/lib/storage` entirely via jest.mock to avoid native module initialization at test time.
- For dual-source hooks (rainviewer + self-hosted), use two `useQuery` calls each with `enabled` flag based on `dataSource` — only one runs at a time.
- Compose `depends_on` cannot reference a service hidden behind a `profiles:` list — validation fails "service X depends on undefined service Y". If the base image is profile-gated, omit `depends_on: [base]` on child services; build-order is enforced by Dockerfile `FROM` + explicit `docker compose build base` step.
- HRRR `.idx` sidecar format: one record per line, colon-delimited: `num:byte_offset:d=YYYYMMDDHH:var_name:level:fcst_time:`. To byte-range subset: fetch the idx, find records whose matcher substrings all appear in the raw line, then issue `Range: bytes=start-end` GETs. Concatenated bytes decode cleanly via pygrib (each GRIB2 record is self-contained).
- Open-Meteo self-hosted: official image is `ghcr.io/open-meteo/open-meteo`, CLI is `openmeteo-api sync <model> <vars> [--past-days N]`, REST endpoint is `/v1/forecast`. Requires a one-time + recurring `sync` invocation to populate model data — we gate that behind a compose profile so it doesn't auto-start.
- Protomaps self-host stack: drop a `.pmtiles` file on disk → serve via `protomaps/go-pmtiles serve <dir>` on :8081 → proxy through Caddy for CORS/caching → point MapLibre style at `/basemap/tiles/{z}/{x}/{y}.mvt`. The style JSONs live in `services/basemap/styles/` and are baked into the tile-server image at `/srv/basemap/styles/`.

## Decision Log
- (2026-04-17) Cumulus redesign adopted from UI-Handoff/design_handoff_cumulus_weather. 3 tabs (Home/Nowcast/Radar), violet #8B7CFF accent, condition gradient backgrounds. Settings → modal via gear icon on Home (keeps self-hosted toggle reachable). Timeline renders past/nowcast/HRRR/long-range segments based on frame unix-time vs now; maps naturally to self-hosted MRMS + HRRR pipeline. New src/lib/cumulusTheme.ts; legacy weatherTheme.ts kept untouched for older WeatherScene.tsx. View-based WeatherIcon (no react-native-svg). New layer `radar-hrrr` added to LAYERS + activeLayer type.
- (2026-04-16) Phase 1 hardening: chose idx-parse byte-range subsetting over adding the `herbie-data` dep. Reasoning: a ~30-line parser is simpler than pulling in herbie's xarray+cfgrib dep chain, and the existing pygrib pipeline keeps working on the subsetted concatenated bytes. Full-file fallback kicks in automatically if the .idx is missing.
- (2026-04-16) Phase 1 self-hosting: new services (open-meteo, basemap) route through the existing Caddy on :8080 rather than exposing extra ports. App only needs one serverUrl. `OPEN_METEO_BASE=http://open-meteo:8080/v1/forecast` in the tile-server so the public path and self-hosted path share the same `/api/forecast/{lat}/{lon}` contract.

## Key Learnings

- Design system v2 (CARROT-style): weather-adaptive gradient backgrounds via getWeatherTheme(), glassmorphism cards (rgba white 0.08-0.15, borderRadius 20), accent #42A5F5, temperature-adaptive font weight via getTempFontWeight(), color-coded temps via getTempColor(). Tab bar semi-transparent rgba(13,17,23,0.95).
- Forecast (Weather tab) is the default hero tab — index.tsx. Radar is second tab (radar.tsx). Settings is third.
- expo-linear-gradient requires native rebuild (not in Expo Go or old dev builds). Must run `npx expo run:android` after adding.
- `size` param from tabBarIcon destructure is unused — safe to omit from destructuring to avoid lint warnings.

## Decision Log

- (2026-04-15) CARROT Weather v2 redesign: Weather-adaptive gradient backgrounds (11 categories × day/night), temperature-adaptive font weight (CARROT's signature), snarky personality quotes, glassmorphism cards, color-coded temps, LinearGradient temp bars in daily forecast, custom View-based tab icons (no emoji). Dependencies: expo-linear-gradient added (requires native rebuild).
- (2026-04-14) CARROT Weather redesign v1: Forecast promoted to hero default tab. Radar moved to dedicated tab. All 4 forecast modal/peek components deleted — forecast fully inlined into the screen. Settings wrapped in card views. ForecastSheet peek-bar pattern abandoned in favor of full-screen forecast tab.

- (2026-04-14) Tasks 6/7: Replaced custom splash/tab layout with QueryClient + GestureHandlerRootView root layout. The old layout used @react-navigation directly; new one delegates tab routing to expo-router's (tabs) group.
- (2026-04-14) Tasks 1-9: Added layer system and self-hosted tile server support. Store persists dataSource/serverUrl to MMKV. useManifest now runs two separate queries with `enabled` guards rather than a single query with conditional queryFn.
- (2026-04-14) Dark theme overhaul: RainViewer COLOR_SCHEME 6 = NEXRAD Level III colors (green/yellow/red/magenta). Tab bar uses position:absolute so map bleeds underneath. bottomControls uses position:absolute with paddingBottom:60 to clear tab bar. ForecastPeek is a translucent bar, not a bottom sheet handle.
- (2026-04-14) UI pro overhaul: Switched free tier from RainViewer to IEM NEXRAD for proper NWS colors. Redesigned bottom bar as two separate absolute-positioned elements (timeline bar at bottom:88, forecast bar at bottom:50). Removed all emoji — text letter icons for layers, label-only tabs. `tabBarShowIcon` not available in expo-router Tabs — use `tabBarIcon: () => null` instead.
