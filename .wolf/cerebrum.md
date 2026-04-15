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
- RainViewer free tier: max zoom 7, Universal Blue color scheme only, past 2hr radar (~13 frames at 10min intervals), nowcast array may be empty.
- No backend needed for Phase 1 — RainViewer (radar tiles), Open-Meteo (forecasts), NWS API (alerts) all free, no auth.
- Expo SDK 55 default template uses `src/app/` (not `app/`) for Expo Router.

## Do-Not-Repeat

- (2026-04-14) Do not assume wolf files exist — check first, create if missing.
- (2026-04-14) In slippy map tile coords, Y increases downward (north = smaller Y). When computing tile range from lat bounds: `tx_min, ty_min = _lat_lon_to_tile(lat_max, lon_min, z)` and `tx_max, ty_max = _lat_lon_to_tile(lat_min, lon_max, z)`. Do NOT swap ty_min/ty_max.
- (2026-04-14) react-native-mmkv v3+ exports `MMKV` as `export type` only — it cannot be instantiated with `new MMKV()`. Use `createMMKV({ id: "..." })` from `react-native-mmkv` instead.
- (2026-04-14) When useManifest returns a union type (RainViewerManifest | SelfHostedManifest), consuming components must use a type guard to access properties specific to one variant (e.g. `manifest.host` only exists on RainViewerManifest).

## Key Learnings

- react-native-mmkv v3+ uses `createMMKV({ id })` factory (not `new MMKV()`). `MMKV` is only a type export.
- Store tests mock `../../src/lib/storage` entirely via jest.mock to avoid native module initialization at test time.
- For dual-source hooks (rainviewer + self-hosted), use two `useQuery` calls each with `enabled` flag based on `dataSource` — only one runs at a time.

## Decision Log

- (2026-04-14) Tasks 6/7: Replaced custom splash/tab layout with QueryClient + GestureHandlerRootView root layout. The old layout used @react-navigation directly; new one delegates tab routing to expo-router's (tabs) group.
- (2026-04-14) Tasks 1-9: Added layer system and self-hosted tile server support. Store persists dataSource/serverUrl to MMKV. useManifest now runs two separate queries with `enabled` guards rather than a single query with conditional queryFn.
