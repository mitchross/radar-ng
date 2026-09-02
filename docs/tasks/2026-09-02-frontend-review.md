# radar-ng frontend review — 2026-09-02

Repo: `/home/vanillax/programming/radar-ng/frontend` @ `6d2ce28` (master). Expo SDK 57, RN 0.86, React 19.2, MapLibre RN 11.3.6, Reanimated 4.5, Skia 2.6.2, react-query 5.101, zustand 5.0.14, MMKV 4.3.2. All paths below are relative to `frontend/` unless absolute.

## Tooling results (run tonight)

| Command | Result |
|---|---|
| `bunx tsc --noEmit` | exit 0, no errors |
| `bun run lint` (`expo lint`) | exit 0, no warnings — but 4 `react-hooks/*` rules are disabled in `eslint.config.js:12-15` (`immutability`, `purity`, `refs`, `set-state-in-effect`), which are exactly the rules that would flag several findings below |
| `bun run test` | 13 suites / 65 tests pass. Coverage is thin (see §6) |

---

## 0. Headline: the "5-slot carousel" was merged and then lost in a merge 14 hours later

The prior review (`docs/tasks/2026-07-04-total-review.md`) said the carousel "has never existed (git log empty)". That is half right. Verified history:

- `b12012f` (2026-07-02 00:19) `perf(app): de-jank radar playback + render hygiene` **added** `frontend/src/lib/radarCarousel.ts`, `frontend/src/hooks/usePlayback.ts`, a `WINDOW = 5` `RadarOverlay.tsx`, `__tests__/lib/radarCarousel.test.ts`, `__tests__/components/RadarOverlay.test.ts`, `__tests__/hooks/useManifest.test.ts`, and split the 1000-line home screen into `components/home/*` + `components/nowcast/*`. (`git ls-tree b12012f -- frontend/src/lib/radarCarousel.ts` → blob `448b6ce`.)
- `80a3727` (01:42) wrote `ARCHITECTURE.md` describing that code.
- `dc1af8c` (15:13) `Merge Weather Clear redesign into master` resolved the frontend by taking the redesign side; `git log -m --first-parent --diff-filter=D -- frontend/src/lib/radarCarousel.ts` names **dc1af8c** as the commit that deleted it. `b12012f` is still an ancestor of HEAD (`git merge-base --is-ancestor` = yes), which is why plain `git log -- <path>` looks empty.

Net: ARCHITECTURE.md § "The app" describes code that is not in the tree. The lost implementation is recoverable with `git show b12012f:frontend/src/lib/radarCarousel.ts` (66 lines, pure, tested) and is the basis for the fix design in §1.

---

## 1. Rendering / playback architecture as it actually exists

### What is there

`src/components/map/RadarOverlay.tsx` (73 lines), mounted from `src/app/(tabs)/radar.tsx:67`:

| Parameter | Value | Where |
|---|---|---|
| Sources mounted | **1** `RasterSource`, id `radar-source` | `RadarOverlay.tsx:55-56` |
| Frame advance | **remount** — `key={`${activePalette}-${layerForUrl}-${frame.path}`}` | `RadarOverlay.tsx:57` |
| Tick interval | **750 ms** (`PLAYBACK_MS`) | `components/timeline/TimelineBar.tsx:17, 77-82` (comment at `:87` still says "420ms tick" — stale) |
| WINDOW / carousel | none (see §0) | — |
| `SOURCE_MIN_ZOOM` | 4 | `RadarOverlay.tsx:28`; `WeatherLayerOverlay.tsx:33` uses `LAYERS[].minZoom` = 4 (`lib/constants.ts:114-123`) |
| `SOURCE_MAX_ZOOM` | radar 7, radar-hrrr 6, nowcast 6; per-frame `frame.maxZoom` from manifest wins | `RadarOverlay.tsx:16-20, 52` (the comment at `:11-15` says "dropped from 9 to 8" — values are 7/6, comment is stale) |
| `raster-fade-duration` | 0 | `RadarOverlay.tsx:68`, `WeatherLayerOverlay.tsx:42`, `home/RadarMiniMap.tsx:97` |
| `tileSize` | 256 | `RadarOverlay.tsx:59` (MapLibre RN default is 512, `RasterSourceNativeComponent.ts:15`) |
| Tile URL template | `${serverUrl}/tiles/${layer}/${palette}/${timestamp}/{z}/{x}/{y}.png` | `lib/tileUrl.ts:10`; `frame.path` is the `${timestamp}` segment |
| Cache-busting | **none** — no query string; identity is the path | `tileUrl.ts`, `useManifest.ts:31-43` |
| Server `Cache-Control` | `public, max-age=86400, immutable` for **all** `/tiles/*` | `backend/api/Caddyfile:8-15`; manifest `max-age=15` at `backend/api/api/server.py:206,210` |

Native facts verified in `node_modules/@maplibre/maplibre-react-native`:
- `RasterSource` has no tiles setter: `ios/components/sources/tile-sources/raster-source/MLRNRasterSource.m` only implements `makeSource` (line 5); JS `RasterSource` freezes `id`. So a URL change **must** remount — the carousel's premise is correct.
- `ios/components/map-view/MLRNMapView.m:233-236` `insertReactSubview:atIndex:` does `[_reactSubviews insertObject:subview atIndex:atIndex]` on an `NSMutableArray` — an index past `count` throws `NSRangeException`. This is the documented crash; it fires when the JS child list churns in a way Fabric's mount indices disagree with the native array (conditional children inside a Fragment). Anything that changes the *number* of children under `<Map>` at runtime touches this path.

### What the user sees today

Every 750 ms tick: unmount source → MapLibre drops that source's in-memory tiles → mount new source → request every visible tile (z4–7 CONUS at city zoom ≈ 12–20 tiles) → PNG decode → GPU upload → paint. With `raster-fade-duration: 0` there is no cross-fade, so the frame **goes blank until the first tiles land, then pops in** — visible flicker on the first loop, and a decode/upload spike on every loop even when the 24 h `immutable` header means the second loop is served from MapLibre's on-disk ambient cache (the network is quiet after loop 1; the CPU/GPU work is not). Scrubbing the `Slider` (`TimelineBar.tsx:205-214`) does the same per step. `WeatherLayerOverlay.tsx:30` (temperature/wind/… layers) and `RadarMiniMap.tsx:86` have the identical remount pattern.

Cache interaction: because MapLibre's ambient cache honours `max-age=86400`, observed radar replay is cheap on the wire. The same header is applied to **every** `/tiles/*` path (`Caddyfile:8-15`), so if any layer's `path` is valid-time-only rather than run-id-qualified (ARCHITECTURE.md claims nowcast/HRRR use 120 s TTL; the Caddyfile does not implement that), a rewritten forecast tile is pinned in the device cache for a day. Backend reviewer should confirm every published `frames[].path` embeds `run_id`; `types/weather.ts:5` comment suggests they do for "model paths" but says nothing about nowcast.

Secondary per-tick costs (all subscribe to `frames`/`currentFrameIndex` and re-render or refetch every 750 ms):
- `components/inspector/Eyedropper.tsx:50-73` — `useEffect` deps include `frame`; while pinned and playing it fires one `/api/inspect` request per tick (~1.3 req/s). Prior M4, still open.
- `components/map/WindParticlesOverlay.tsx:88-97` — `useWindField(timestamp)` keyed on the current frame's timestamp; on the wind layer each tick is a new query key → full u/v grid download per new frame (staleTime 15 min caps it to one pass per loop). Prior M4, still open.
- `components/map/LayerLocationMarker.tsx:29-30, 36-49` — subscribes to `frames` + `currentFrameIndex` unconditionally (only needed for AQ layers) so the `<Marker>` re-renders every tick on every layer.
- `components/map/LayerLegendCard.tsx:155-156` — same; only needs the current frame's `source`.

### Fix design (restore the carousel, hardened)

Recover `b12012f`'s `assignSlots`/`clampWindow` (pure, 66 lines, has tests) and the slot-mapped `RadarOverlay`. Constraints to keep: constant child count under `<Map>`, unique `id` per (slot, frame), opacity swap for visibility, and a `WINDOW = 1` kill switch.

```tsx
// src/lib/radarCarousel.ts  — restore verbatim from `git show b12012f:frontend/src/lib/radarCarousel.ts`
// src/components/map/RadarOverlay.tsx
const WINDOW = 5;  // 1 = old behaviour (kill switch)

export function RadarOverlay() {
  const frames = useWeatherStore((s) => s.frames);
  const idx = useWeatherStore((s) => s.currentFrameIndex);
  const { start, end } = useWeatherStore((s) => s.playbackWindow) ?? { start: 0, end: frames.length - 1 };
  // ...palette/opacity/serverUrl selectors as today
  if (frames.length === 0 || idx < 0) return null;

  const { slots, visibleSlot } = assignSlots(Math.min(idx, frames.length - 1), start, end, WINDOW);
  return slots.map((frameIndex, slot) => {
    const frame = frames[frameIndex] ?? frames[idx];          // never null → child count constant
    const layer = frame.source ?? activeLayer;
    const id = `radar-${slot}-${activePalette}-${layer}-${frame.path}`;
    return (
      <RasterSource key={id} id={id} tiles={[buildSelfHostedTileUrl(serverUrl, layer, frame.path, activePalette)]}
                    tileSize={256} minzoom={SOURCE_MIN_ZOOM} maxzoom={frame.maxZoom ?? SOURCE_MAX_ZOOM[layer] ?? 7}>
        <Layer type="raster" id={`${id}-layer`}
               paint={{ "raster-opacity": slot === visibleSlot && radarVisible ? radarOpacity : 0,
                        "raster-fade-duration": 0 }} />
      </RasterSource>
    );
  });
}
```

Notes for the restore:
- `TimelineBar` must publish its `{startIdx,endIdx}` window to the store (`b12012f` added `playbackWindow` + `usePlayback.ts`), otherwise the prefetch slots aim at the wrong frames when zoom = 1h.
- A `+1` advance changes exactly one slot (the one that just played), so each remount happens hidden with `(WINDOW-1) × 750 ms ≈ 3 s` of prefetch time — comfortably more than the 1.7 s the original design had at 430 ms.
- Opacity is a paint update on a live layer (`MLRNSource.m:28/42` add/remove is only on mount/unmount) — no remount, no blank flash.
- iOS risk: this changes the child count under `<Map>` from 1 to 5, and today other children already toggle at runtime (`AlertPolygon.tsx:41-44`, `LightningOverlay.tsx:11`, `StormCellsOverlay.tsx:11`, `TropicalOverlay.tsx:25`, `EyedropperPin` via `radar.tsx:83`). Ship behind `WINDOW`, test on a physical iPhone with alerts present and extras toggled. Consider making `AlertPolygon`/`Lightning`/`StormCells` always render their `GeoJSONSource` with an empty `FeatureCollection` instead of returning `null` so the `<Map>` child count is truly constant — that removes the NSRange trigger for good. (`WeatherMap.tsx:183` already filters children through `Children.toArray().filter(isValidElement)`, so position keys are stable; count is what varies.)
- Apply the same slot pattern to `WeatherLayerOverlay` (or generalise `RadarOverlay` to take a `layerId`) — it is a copy of the remount design.

---

## 2. Re-render audit

Store: `src/stores/useWeatherStore.ts` — plain `create()` without `subscribeWithSelector`/`useShallow`; every consumer uses single-field selectors (good). `setFrames` (`:123`) is called from `useManifest.ts:135-137` with a **new array on every manifest poll** because `buildSelfHostedFrames` (`useManifest.ts:19-59`) calls `Date.now()` (`:46`) and builds fresh objects, and react-query returns a new `query.data` whenever `updated_at` changes (every 15–30 s). Every `frames` subscriber re-renders on every poll: `RadarOverlay:36`, `WeatherLayerOverlay:13`, `TimelineBar:36` (recomputes three `findClosestIdx` scans, `:52-62, 90-101`), `WindParticlesOverlay:84`, `LayerLocationMarker:29`, `LayerLegendCard:156`, `Eyedropper:41`. Fix: derive frames once per (manifest.updated_at, layer, mode) and only `setFrames` if a cheap structural compare (length + first/last `path`) differs.

Map screen (`src/app/(tabs)/radar.tsx`):
- `:60-65` inline arrow props to `WeatherMap` are fine (they write shared values), but `onCameraChanged` is driven by `onRegionIsChanging` (`WeatherMap.tsx:173`) — a JS-thread event stream at up to 60 Hz while panning, only to copy three numbers into Reanimated shared values. Acceptable, but it is the one place JS is on the hot path during gestures.
- `:104` `LayerLegendCard` and `:82` `LayerLocationMarker` re-render every tick (see §1).
- `WeatherMap.tsx:135-142` effect calls `cameraRef.setStop` whenever `centerCoord` changes — every GPS fix recenters the map under the user, including mid-pan. Also `useLocation` is mounted in four screens (`radar.tsx:40`, `index.tsx:44`, `alerts.tsx:21`, `NowcastScreen.tsx:36`); each mounted tab runs its own permission + `getCurrentPositionAsync` (10 s) sequence (`hooks/useLocation.ts:28-89`).

Timeline (`components/timeline/TimelineBar.tsx`): subscribes to `frames` + `currentFrameIndex` (`:36-37`) so the whole card, both `Pressable` groups, both `DashedRow`s (24 Views) and the native `Slider` re-render each tick. Cheap per render, but `toLocaleDateString` (`:109-113`) constructs an `Intl.DateTimeFormat` per tick — hoist a formatter (skill rule `js-hoist-intl`). Inline style objects at `:138, 168, 175, 184, 195, 203, 231, 248`.

Home tab (`src/app/(tabs)/index.tsx`, 1096 lines, single component):
- All derived data (`hourly`, `daily`, stats, `todayLocal`) is computed inline in the render body (`:110-190`) with no `useMemo`; the screen re-renders on every `useAlerts` 60 s poll, every `useForecast` 15 min poll, every store change it subscribes to, and every `RefreshControl` state flip.
- `:318-340` hourly strip is a horizontal `ScrollView` of 24 cells each containing a `WeatherIcon` (View-composed, `components/weather/WeatherIcon.tsx:9`) — ~24 × ~10 Views; `:378-427` 7-day rows each with `LinearGradient`. No virtualization anywhere (no FlatList/FlashList/LegendList in `src/`). For 24+7 items this is acceptable; the cost is the mini map:
- `:429-432` `RadarMiniMap` mounts a **second full MapLibre GL surface** inside the ScrollView (`components/home/RadarMiniMap.tsx:69-102`), runs its own style fetch (`:36`) and its **own manifest poll** with a different key `["manifest", serverUrl, "mini"]` at 60 s (`:38-43`) — no dedup with the radar tab's `["manifest", serverUrl]`, and the Settings tab has a third `["manifest", serverUrl]` query without `refetchInterval` (`settings.tsx:105-109`). Three manifest queries for one URL.
- `:47` `createStyles(theme)` memoised — good. `:218` `as any` route push.
- Stat widgets (`components/home/StatWidgets.tsx`) are 4 Skia `Canvas` instances (`grep -c "<Canvas"` = 4) in Advanced mode — fine.

Alerts (`src/app/(tabs)/alerts.tsx`): plain `ScrollView` + `alerts.map` (`:62-98`), each `AlertCard` calls `useMemo(() => createStyles(theme))` **per card** (`:108`) — recreates a full `StyleSheet.create` per item on theme change and holds N copies. Hoist to module/screen scope. Inline `boxShadow` per card (`:122-126`). Alert counts are small (<20) so no virtualization needed, but `RefreshControl refreshing={isFetching}` (`:68`) spins on every background 60 s poll, not just pulls.

Tab bar (`src/app/(tabs)/_layout.tsx:30-36`): custom JS `tabBar` (not `NativeTabs`, skill `navigation-native-navigators`) subscribes to `useAlerts()` so the bar re-renders on each alerts poll; `onPress` closures recreated per render (`:48-57`). Minor.

Context provider: `theme/WeatherClearThemeProvider.tsx:22-28` memoises the value — correct.

Skia wind particles (`components/map/WindParticlesOverlay.tsx`): the loop is genuinely on the UI thread (`useFrameCallback` `:140-179`, `useDerivedValue` `:186-299`), but each frame builds **four** separate `Skia.Path` objects, each iterating all 1200 particles and re-projecting (`:194-211` ×4), and the canvas draws 8 `Path` strokes (`:311-318`). That is 4× the projection work needed; build one pass that appends to four paths, or bucket by speed once. `PARTICLE_COUNT=1200` at 60 fps on the UI thread will show as dropped frames on mid-range Android while also panning. `runOnUI` reseed at `:118-138` re-runs whenever `field` identity changes (new query data) — fine.

Reanimated vs RN `Animated`: `components/map/RadarFABs.tsx:236-262` uses legacy `Animated.loop` with `useNativeDriver` for the refresh spinner; `lib/animatedFix.ts` monkey-patches `AnimatedNode.__callListeners` at import time (`_layout.tsx:3`) with a comment referencing RN 0.83 — verify it is still needed on 0.86 before it silently masks a real listener bug.

`react-query select` misuse: none (no `select` used anywhere). Zustand selectors returning new objects: none.

---

## 3. Data layer

Flow is as documented: `hooks/* (useQuery) → useWeatherStore → components`, except `useManifest` is the only hook that writes to the store; the others are consumed directly.

QueryClient (`src/app/_layout.tsx:31-53`): `retry: 2`, `gcTime: 5 min`, default `staleTime: 0`, default `refetchOnWindowFocus`. **No `focusManager`/`onlineManager` wiring** — in RN the focus manager is never told about `AppState`, so (a) `refetchOnWindowFocus` never fires on foreground, and (b) `refetchIntervalInBackground: false` (default) has no effect because RN never reports "unfocused". `grep AppState src` → zero hits.

Polling cadences vs server:

| Hook | Interval | staleTime | Server | Notes |
|---|---|---|---|---|
| `useManifest` `hooks/useManifest.ts:107-128` | 30 s | 0 | 15 s `max-age` | `initialData` from MMKV, `initialDataUpdatedAt` not set → cached manifest is treated as **fresh** (defaults to now), so the immediate refetch ARCHITECTURE promises only happens because `refetchInterval` fires 30 s later. Set `initialDataUpdatedAt: 0`. |
| `RadarMiniMap.tsx:38-43` | 60 s | 30 s | 15 s | separate key, see §2 |
| `settings.tsx:105-109` | none | 30 s | 15 s | third key |
| `useAlerts` `:14-15` | 60 s | 60 s | NWS | key includes full-precision lat/lon (`:11`) → every GPS jitter is a new query + NWS request |
| `useForecast` `:15-16` | 15 min | 15 min | 300 s | ok |
| `useRadarNowcast` `:16-18` | 60 s | 60 s | ? | ok |
| `useLightning` `:33-34` | **10 s** | 8 s | — | keeps polling while the user is on Home (radar tab stays mounted) whenever `extrasVisible` |
| `useStormCells` `:39-40` | 60 s | 60 s | — | same |
| `useTropical` `:38-39` | 5 min | 5 min | — | ok |
| `useWindField` `:47-49` | off | 15 min | — | per-frame key, §1 |
| `useStormTilePrefetch` `:31-33` | 5 min | 5 min | — | root layout, gated by `extrasVisible` |
| `settings.tsx:99-103` `server-status` | 60 s | 0 | — | only while Settings mounted |

Background battery: expo-router `Tabs` keeps visited screens mounted; once the Radar tab has been opened, `useManifest` (30 s), and with extras on `useLightning` (10 s) + `useStormCells` (60 s), keep polling while the user is on Home. Android keeps the JS timers alive with the app backgrounded until the process is killed; iOS suspends after ~30 s. Fix: `focusManager.setEventListener` with `AppState` in `_layout.tsx`, and pause playback + heavy polls when `AppState !== "active"` (also stops the 750 ms interval spinning in background).

503/degraded: `lib/api.ts:103-114` `checkServerHealth` returns `res.ok`, and `/api/health` answers **503** when degraded (per `fetchServerStatus` comment `:130-134`). Production is degraded right now, so the Settings → Data Sources card shows every source as `ERROR` (`settings.tsx:495-541` derives one status for all rows from that boolean) even though tiles are serving. Use `fetchServerStatus().status` and per-source truth instead.

Retry/backoff: react-query default exponential (1 s, 2 s) ×2 — fine for 503. But **no request timeouts**: `fetchSelfHostedManifest` (`api.ts:73`), `fetchForecast` (`:20`), `fetchAlerts` (`:58`), `useLightning/useStormCells/useTropical/useWindField` all call bare `fetch()`; only the health calls use `AbortSignal.timeout(5000)` (`api.ts:106,138`). A hung Cloudflare hop stalls a query until the OS socket timeout. No `AbortController` is threaded from react-query's `signal` in any `queryFn` (unmount does not cancel in-flight requests; harmless for GETs, wasteful on cellular).

Offline: only the manifest is persisted (`useManifest.ts:106-126`, key `manifest-cache-v2`). Forecast/alerts are not, so an offline cold start on the Home tab shows the error `ScreenState` (`index.tsx:84-96`) rather than the last forecast. Consider `@tanstack/query-persist-client` with an MMKV persister for `forecast`/`alerts`/`radar-nowcast`. Also `storage.ts:5-17` swallows MMKV exceptions (good), but the store casts persisted strings without validation — `useWeatherStore.ts:112-120` `getString("mapStyle") as MapStyle` etc.; a bad `mapStyle` value reaches `resolveMapStyleUrl` → `MAP_STYLES_SELFHOSTED[mapStyle]` is `undefined` → `path.startsWith` throws at `lib/constants.ts:71` → red screen at `WeatherMap` mount. Prior Low, still open; add parsers like `parseAppearanceMode` (`:79-81`).

Dedup: `useAlerts()` is called in 5 places (`_layout` tab bar, home, alerts, `AlertPolygon`, `alert/[id]`) — same key, deduped correctly. Manifest is **not** (three keys, above).

Error boundaries: single root `export { ErrorBoundary } from "expo-router"` (`_layout.tsx:29`). No per-tab boundary; a render error on Home replaces the whole app including the tab bar. Query errors are only logged (`_layout.tsx:41-52`).

`useLocation.ts:44-83`: `requestForegroundPermissionsAsync()` (`:50`) sits outside any try/catch and `requestLocation()` is not awaited (`:85`) — a throw there is an unhandled rejection. Selected-city mode sets coordinates on every effect run (`:45-48`) — fine.

---

## 4. Startup and bundle

Cold-start path: `expo-router/entry` → `src/app/_layout.tsx`. Module-scope work before first paint:
1. `lib/animatedFix.ts` prototype patch (`_layout.tsx:3`).
2. `lib/telemetry.ts` — imports **the full OTel web SDK** (`sdk-trace-web`, `sdk-logs`, two OTLP exporters, resources, semconv; `telemetry.ts:10-32`) and builds a `Resource` (`:41-49`) even when `TELEMETRY_ENABLED` is false (`:38-39`). The providers are only registered when enabled (`:51-74`), but the bundle and module-init cost are paid by every user. Make the SDK import lazy (`await import(...)` inside the enabled branch) and keep `trace()`/`logEvent()` as no-op shims.
3. `useWeatherStore.ts:92-121` — synchronous MMKV reads at module init (cheap, correct).
4. `QueryClient` construction, `GestureHandlerRootView`, `SafeAreaProvider`, theme provider, `useStormTilePrefetch` (dynamic-imports MapLibre only when a plan exists — good, `useStormTilePrefetch.ts:46`).
5. First tab = Home: `index.tsx` statically imports `RadarMiniMap` → `@maplibre/maplibre-react-native` and `StatWidgets` → `@shopify/react-native-skia` at module scope (`index.tsx:33-41`). So MapLibre native init + a GL surface + a style fetch + a manifest fetch all happen on the Home first paint, before the user has asked for a map. Lazy-load `RadarMiniMap` (render a static placeholder until the forecast has painted, or replace with a static PNG tile composite).

Fonts: embedded natively via the `expo-font` config plugin (`app.json:66-79`) — no `useFonts` gate, good. Splash: plugin configured (`app.json:41-49`), no manual `preventAutoHide` — fine.

Hermes / New Architecture: `android/gradle.properties` `newArchEnabled=true`, `hermesEnabled=true`, `edgeToEdgeEnabled=true`; no `ios/` checked in (CNG/prebuild); SDK 57 is New-Arch-only. `app.json:86-89` `reactCompiler: false` — enabling it would mechanically fix most of §2's missing memoisation once the disabled `react-hooks` lint rules are cleaned up.

`metro.config.js`: default config, nothing tuned (no `inlineRequires` override needed — Expo default enables it).

iOS deployment target is **26.0** (`app.json:55`) with a custom pod-floor plugin (`plugins/withXcode27PodDeploymentFloor.js`) — this excludes every device not on iOS 26; confirm that is intentional.

Assets: `assets/images/icon.png` 1.38 MB and `assets/expo.icon/Assets/grid.png` 1.43 MB (icon-composer source, fine), `assets/images/logo-glow.png` 331 KB is **unreferenced** (0 hits in `src/`), `android-icon-background.png` 371 KB. `scripts/reset-project.js` is the Expo template leftover.

Per-render module cost: `WeatherIcon` is View-composed (no SVG/Skia) — cheap; `expo-linear-gradient` used per daily row (`index.tsx:401-413`) — acceptable.

---

## 5. Correctness bugs

1. **Timeline "now" frozen at mount** — `TimelineBar.tsx:49` `useMemo(() => Date.now(), [])`; after 30+ min on the tab the 1 h window, NOW marker and "Now/Past/Forecast" label (`:106,115`) drift into the past. Prior M1, still open. Also `useManifest.ts:46` bakes `nowSec` into the frame list at poll time (refreshes every 30 s, so tolerable), and `pickNowFrameIndex` recomputes (`:80`).
2. **Frame identity is a bare index across polls** — `useManifest.ts:144-149` only re-snaps when index is -1 or out of range; a poll that prunes the head moves the paused/playing frame forward in time silently. Prior M2, still open. `TimelineBar.tsx:65-72` snap effect has `frames.length` in deps but a pruned-head poll keeps length constant.
3. **Health boolean misread** — `api.ts:103-114` treats 503-degraded as down (see §3); today's production state makes the Settings screen lie.
4. **Fabricated UI values still shipped** — `settings.tsx:186-190` `UPTIME 14d / TILES/DAY 48.2k / CACHE 87%` are hard-coded; `alerts.tsx:177,181` `MIZ064 · Kent` and `Last polled 2 min ago` are hard-coded; `settings.tsx:80` `refreshLabel "2 min"` state is never derived. Prior Low ("fabricated stats"), partially open.
5. **Dead settings** — Playback FPS slider (`settings.tsx:434-440`) writes `playbackSpeed`, which nothing reads except the unmounted `PlayButton.tsx:9`; `TimelineBar` hard-codes 750 ms. °F/°C toggle (`settings.tsx:403-407`) is cosmetic (`LayerLocationMarker.tsx:109-114` comment admits it). Flat/Globe projection (`MapStylePicker.tsx:54-77`) is a no-op (`:6-7` comment). Prior Low, still open.
6. **Persisted-value casts can crash** — `useWeatherStore.ts:112-120` (see §3).
7. **Hourly strip night flag uses today's sunrise/sunset for tomorrow's hours** — `index.tsx:139-140` `hr < sunrise || hr > sunset` with `daily.sunrise[0]`; hours after midnight render as "night" using the wrong day (prior Low, still open). The 7-day weekday bug was fixed (`index.tsx:161-167` appends `T00:00:00`).
8. **`useLocation` unhandled rejection path** — `useLocation.ts:50,85` (see §3). `withTimeout` (`:12-19`) leaves the losing timer running (harmless leak of one 10 s timer).
9. **Hard-coded author defaults** — `lib/constants.ts:92` `DEFAULT_URL: "https://radar-ng-api.vanillax.me"`, `:82-83` Grand Rapids coordinates, `useWeatherStore.ts:83-90` `DEFAULT_PLACE` Grand Rapids. Fine for a personal build; should be build-time env (`EXPO_PUBLIC_RADAR_SERVER_URL`) for forks.
10. **Telemetry** — now opt-in (`telemetry.ts:37-39`, requires `EXPO_PUBLIC_TELEMETRY_ENABLED=1` + `EXPO_PUBLIC_OTLP_BASE`) and `privacySafeAttributes` (`:79-88`) strips `geo.lat/geo.lon` (`api.ts:25,46,65`, `inspector.ts:54-55`). Prior M5 **fixed**. Remaining gap: `inspector.timestamp`/`inspector.value` and `error.stack` (`:155`) still export; `docs/configuration.md` documents the basemap `EXPO_PUBLIC_*` vars but not the telemetry ones.
11. **Stale comments contradict code** — `RadarOverlay.tsx:11-15` ("dropped to 8") vs values 7/6; `TimelineBar.tsx:5` ("750ms") vs `:87` ("420ms tick"); `animatedFix.ts:1` references RN 0.83 on RN 0.86; ARCHITECTURE.md § "The app" (§0).
12. `Eyedropper.tsx:64-70` `.then().finally()` with no `.catch` — `inspectPoint` never rejects (`inspector.ts:46-49`), so safe today, but fragile.
13. `alerts.tsx:96` builds the route with `encodeURIComponent(alert.id)` while `index.tsx:293-297` passes `properties.id` as a param; `alert/[id].tsx:29` matches either — ok, but two conventions for one route.
14. `RadarMiniMap.tsx:36` always requests the `"light"` style regardless of the user's `mapStyle`/dark theme.

Timezone/date parsing otherwise sound: ISO strings via `new Date()` (`useManifest.ts:32`, `alerts.tsx:110`, `TropicalDetailSheet.tsx:98-107`); `formatHour` (`index.tsx:584-590`) is local-time; `radarNowcast.ts:14-19` mm/h→in/h correct; `index.tsx:187` m→mi correct.

---

## 6. Code quality / structure

Layout: `src/app` (expo-router), `components/{map,timeline,home,alerts,inspector,layers,palette,ui,weather}`, `hooks`, `lib`, `stores`, `theme`, `types`, `screens` (one file). Reasonable, but three god-files: `app/(tabs)/settings.tsx` 1366 lines, `app/(tabs)/index.tsx` 1096, `screens/NowcastScreen.tsx` 868, plus `components/map/RadarFABs.tsx` 735 (mostly View-drawn icons). `b12012f` had already split Home/Nowcast into `components/home/*` and `components/nowcast/*`; that split was lost in `dc1af8c` too.

Dead code (zero importers, verified with grep):
- `components/timeline/PlayButton.tsx` (own 200 ms interval — would double-tick if ever mounted), `components/timeline/TimeSlider.tsx`, `components/layers/LayerPicker.tsx`, `components/alerts/AlertBanner.tsx`.
- `components/weather/WeatherScene.tsx` (485 lines) and its only dependency `lib/weatherTheme.ts` (395 lines) — 880 lines unreferenced.
- `components/animated-icon.module.css`, `global.css` (web leftovers; platforms are ios/android only, `app.json:10-13`).
- Store: `visibleOverlays`/`toggleOverlay` (only `LayerPicker`), `nextFrame` (only `PlayButton`), `setRadarVisible` never called (`radarVisible` is always `true`), `playbackSpeed` (see §5.5).
- `lib/constants.ts:74-79` `RADAR.*` unused; `SELF_HOSTED.METRICS_PATH` never fetched; `lib/inspector.ts:108-116` `gridUrlFor` self-described unused; `lib/storage.ts:19-32` `getBoolean/setBoolean` unused; `api.ts:103-114` `checkServerHealth` duplicates `fetchServerStatus`.
- Push/watch UI for the server's `DISABLE_WORKFLOW_ROUTES=1` routes: **none exists in the app** (`grep -i "push-token|/v1/|watch"` finds only alert-kind classification) — the frontend never calls `/v1/*`, so nothing to remove there.

Typing: `strict: true`; two `as any` (`alerts.tsx:96`, `index.tsx:218`) for typed routes; ~25 `as never` casts for MapLibre expressions (`AlertPolygon`, `LightningOverlay`, `StormCellsOverlay`, `TropicalOverlay`) — a typing gap in `@maplibre/maplibre-react-native`'s expression types; non-null `latitude!` in hooks is guarded by `enabled`.

Tests (`__tests__`, jest + ts-jest, `testEnvironment: node`): 13 suites / 65 tests — pure `lib/*` (frameIndex, tileUrl, geocoding, locationLabel, weatherCodes, weatherPresentation, radarNowcast, mapStyle, api), the store, and theme. The two "component" suites (`WeatherMap.test.ts`, `weatherClearContracts.test.ts`) are **source-text greps** (`readFileSync` + `toContain("minHeight: 44")`) — they assert on strings in files, not behaviour, and break on any refactor. Not covered: `useManifest` frame merging/snap, `RadarOverlay`, `TimelineBar` window/tick, any hook, any render. (`b12012f` had `useManifest.test.ts`, `RadarOverlay.test.ts`, `radarCarousel.test.ts` — also lost.) `jest-environment-jsdom` and `test-mocks/react-native.ts` (4 lines) are present but unused. No RN Testing Library.

Lint: passes with four `react-hooks` rules disabled (`eslint.config.js:12-15`); `react-hooks/exhaustive-deps` suppressed once (`TimelineBar.tsx:71`).

Duplicated logic: `createStyles(theme)` factories per screen and per `AlertCard`; manifest→frames mapping exists twice (`useManifest.ts:24-44`, `RadarMiniMap.tsx:53-60`); NWS severity colour maps exist in `AlertPolygon.tsx:14-28`, `alerts.tsx:189-200`, `alert/[id].tsx`; `SOURCE_MAX_ZOOM`/`MIN_ZOOM` constants repeated in `RadarOverlay`, `RadarMiniMap:25-26`, `constants.ts:113-123`.

---

## 7. Ranked plan (user-visible impact first)

### Safe tonight (no native child-count change, no device dependence)

| # | Change | Effort | Files |
|---|---|---|---|
| 1 | Wire `focusManager` to `AppState` and pause playback + lightning/storm polls when not `active`; also `refetchOnWindowFocus` starts working | S | `app/_layout.tsx:31`, `TimelineBar.tsx:75-84`, `useLightning.ts`, `useStormCells.ts` |
| 2 | Fix stale "now": replace `TimelineBar.tsx:49` memo with a 60 s-refreshing `nowSec`; have `useManifest` re-snap by frame `time` instead of index (`:144-149`) | S | `TimelineBar.tsx`, `useManifest.ts` |
| 3 | Stop per-tick fetch storms: debounce `Eyedropper` effect to fire only when `isPlaying` is false or 400 ms after last change (`:50-73`); key `useWindField` on the HRRR run/hour, not per frame (`WindParticlesOverlay.tsx:88-97`) | S | 2 files |
| 4 | Single manifest query: make `RadarMiniMap` and Settings consume `["manifest", serverUrl]` (`RadarMiniMap.tsx:38-43`, `settings.tsx:105-109`); set `initialDataUpdatedAt: 0` in `useManifest.ts:114` | S | 3 files |
| 5 | Read `/api/health` JSON status instead of `res.ok` (`api.ts:103-114`, `settings.tsx:85-94, 495-541`); delete fabricated stats (`settings.tsx:186-190`, `alerts.tsx:177-181`) | S | 2 files |
| 6 | Validate persisted MMKV values (`useWeatherStore.ts:112-120`) with parsers → removes a real crash path | S | 1 file |
| 7 | Add `AbortSignal.timeout(10_000)` (or react-query `signal`) to every `fetch` in `api.ts`, `useLightning/StormCells/Tropical/WindField` | S | 5 files |
| 8 | Delete dead code (§6 list, ~1.5 k lines incl. `WeatherScene`+`weatherTheme`); remove Playback-FPS slider or make `TimelineBar` honour `playbackSpeed`; fix stale comments (§5.11); regenerate ARCHITECTURE § "The app" to match reality | S–M | many, all deletions |
| 9 | Lazy-import the OTel SDK inside the enabled branch of `telemetry.ts`; lazy-load `RadarMiniMap` behind the first forecast paint | M | `telemetry.ts`, `index.tsx:33,429` |
| 10 | Memoise per-poll frame derivation (skip `setFrames` when unchanged) and hoist `Intl` formatters / per-card `createStyles` | M | `useManifest.ts:130-137`, `TimelineBar.tsx:109`, `alerts.tsx:108` |

### Needs on-device testing (touches MapLibre child count / native)

| # | Change | Effort | Risk |
|---|---|---|---|
| A | **Restore the 5-slot carousel** from `b12012f` (`radarCarousel.ts` + tests verbatim, `RadarOverlay` per §1 sketch, `playbackWindow` in store fed by `TimelineBar`) behind `WINDOW`; apply to `WeatherLayerOverlay` too | M | Child count 1→5 under `<Map>`; iOS `MLRNMapView.m:233-236` NSRange path — test on a physical iPhone with alerts present, extras toggled, layer switches, palette switches, 1h↔48h zoom, server URL change |
| B | Make `AlertPolygon`/`LightningOverlay`/`StormCellsOverlay`/`TropicalOverlay` always mount their `GeoJSONSource` (empty collection instead of `null`) so `<Map>` child count is constant regardless of data | S | Same native path; removes the runtime churn that exists today |
| C | Move camera bridging off `onRegionIsChanging` (JS 60 Hz) — either throttle to `onRegionDidChange` for particles or accept; and stop recentering on every GPS fix mid-pan (`WeatherMap.tsx:135-142`) | S | Gesture feel; test on device |
| D | Wind particles: one projection pass feeding four paths; consider `PARTICLE_COUNT` 800 on Android | S | UI-thread FPS on mid-range Android |
| E | Replace the JS custom tab bar with expo-router `NativeTabs` (skill: native navigators) | M | Tab bar visuals/badge; per-platform QA |
| F | Enable React Compiler (`app.json:88`) after re-enabling the four `react-hooks` lint rules and fixing what they flag (`RadarFABs.tsx:236` ref-in-render, `useLocation.ts` set-state-in-effect) | L | Whole-app render behaviour; full regression pass |

### Prior-review status (frontend items)

Fixed: M3 (frames rebuild on layer/server switch, `useManifest.ts:130-133`), M5 (telemetry opt-in + PII filter), 7-day weekday UTC bug (`index.tsx:161-167`), `findClosestIdx` empty guard (`lib/frameIndex.ts:14`).
Still open: M1 (`TimelineBar.tsx:49`), M2 (`useManifest.ts:144-149`), M4 (`Eyedropper.tsx:50-73`, `WindParticlesOverlay.tsx:88-97`), dead settings (§5.5), fabricated stats (§5.4), unvalidated MMKV casts (§5.6), hourly night flag (§5.7). New: the carousel loss (§0), all-`/tiles/*` 24 h immutable (§1), no AppState handling (§3), health-503 misread (§3), no fetch timeouts (§3), Home mounts a second GL map (§4).
