# Project Anatomy — radar-ng

Expo SDK 55 React Native weather radar app (StormScope). Uses expo-router for file-based routing, Zustand for state, React Query for data fetching, MapLibre for map rendering.

## Configuration

- `package.json` — dependencies: expo ~55, @tanstack/react-query ^5, expo-location, expo-router, zustand ^5, react-native-gesture-handler, maplibre-react-native. ~60 tokens.
- `tsconfig.json` — TypeScript config. ~20 tokens.
- `jest.config.js` — ts-jest preset, testEnvironment node, roots: __tests__, moduleNameMapper @/ → src/. ~30 tokens.
- `app.json` — Expo app config. ~40 tokens.

## Source: App Routes (expo-router)

- `src/app/_layout.tsx` — Root layout. Sets up GestureHandlerRootView, QueryClientProvider (retry:2, gcTime:10min), StatusBar light, Stack with (tabs) and alert/[id] (modal). ~50 tokens.
- `src/app/(tabs)/_layout.tsx` — Tab navigator. Dark theme (#1a1a2e bg), Map (🗺️ emoji) and Settings (⚙️ emoji) tabs with Unicode icon Text components, height 56, paddingBottom 6, label fontSize 11. ~55 tokens.
- `src/app/(tabs)/index.tsx` — Map screen. Calls useLocation + useManifest. Reads activeLayer/visibleOverlays/radarOpacity/dataSource from store. Renders WeatherMap with RadarOverlay, WeatherLayerOverlay (self-hosted layers), AlertPolygon, AlertBanner, LayerPicker, timeline bar, ForecastSheet. ~70 tokens.
- `src/app/(tabs)/settings.tsx` — Full Settings screen. SafeAreaView, Section/Row/SegmentedControl helpers. Controls: map dark mode, temp unit, radar opacity, playback speed, data source toggle (Free/Self-Hosted), server URL TextInput (shown when selfhosted). Footer attribution. ~230 tokens.
- `src/app/alert/[id].tsx` — Alert detail modal screen. Uses useLocalSearchParams to get id. ~30 tokens.

## Source: Hooks

- `src/hooks/useManifest.ts` — Dual-source manifest hook. Runs rainviewerQuery (enabled when dataSource=rainviewer) and selfHostedQuery (enabled when dataSource=selfhosted). Syncs frames to store. Returns active query. ~90 tokens.
- `src/hooks/useForecast.ts` — React Query hook for Open-Meteo forecast. Enabled when lat/lon set. refetchInterval: 15min. ~35 tokens.
- `src/hooks/useAlerts.ts` — React Query hook for NWS alerts. Enabled when lat/lon set. refetchInterval: 60s. ~35 tokens.
- `src/hooks/useLocation.ts` — expo-location hook. Requests foreground permission, falls back to DEFAULTS lat/lon on denial. Sets location in Zustand. ~45 tokens.

## Source: Lib

- `src/lib/api.ts` — fetchRadarManifest, fetchForecast, fetchAlerts, fetchSelfHostedManifest, checkServerHealth. ~110 tokens.
- `src/lib/constants.ts` — API URLs, MAP_STYLES, RADAR config, DEFAULTS, SELF_HOSTED, LAYERS (LayerConfig[]). ~70 tokens.
- `src/lib/storage.ts` — MMKV wrapper using createMMKV({id:"stormscope"}). getString, setString, getBoolean, setBoolean helpers. ~40 tokens.
- `src/lib/tileUrl.ts` — buildRadarTileUrl (RainViewer), buildSelfHostedTileUrl (self-hosted). ~50 tokens.
- `src/lib/weatherCodes.ts` — WMO weather code descriptions. ~60 tokens.

## Source: Stores

- `src/stores/useWeatherStore.ts` — Zustand store. State: frames, currentFrameIndex, isPlaying, lat/lon, radarOpacity/Visible, activeLayer, visibleOverlays, temperatureUnit, mapStyle, dataSource, serverUrl. Persists dataSource+serverUrl to MMKV via storage.ts. ~120 tokens.

## Source: Components

- `src/components/map/WeatherMap.tsx` — MapLibre MapView wrapper. Reads mapStyle/lat/lon from Zustand, renders Camera + UserLocation. Accepts children for overlays. Sets access token null. ~50 tokens.
- `src/components/map/RadarOverlay.tsx` — Radar tile overlay. useMemo to compute tileUrl from manifest+frame. key={tileUrl} on RasterSource (not frame.path). maxZoomLevel 12 for selfhosted, RADAR.MAX_ZOOM for rainviewer. isRainViewerManifest() type guard. ~75 tokens.
- `src/components/timeline/TimeSlider.tsx` — Radar timeline slider. Reads frames/currentFrameIndex from Zustand, renders @react-native-community/slider. Shows time label, LIVE badge (green pill), ago label (e.g. "5m ago"), and frame counter (N/total). Pauses playback on drag. thumbTintColor #fff, maximumTrackTintColor #444. ~65 tokens.
- `src/components/timeline/PlayButton.tsx` — Play/pause button. Uses setInterval at playbackSpeed fps to call nextFrame when playing. Styled with CSS triangle (play) and two rects (pause) — no emoji. Button bg #4fc3f7, size 48x48. ~55 tokens.
- `src/components/forecast/CurrentConditions.tsx` — Current weather card. Renders temp, condition icon/label, H/L, feels-like, wind, humidity, gusts from OpenMeteoResponse. ~50 tokens.
- `src/components/forecast/HourlyScroll.tsx` — Horizontal 24-hour forecast scroll. Shows icon, temp, precip%, wind per hour from OpenMeteoResponse. ~50 tokens.
- `src/components/forecast/ForecastSheet.tsx` — Custom 3-state sheet (collapsed/half/full). Pressable handle with 44pt touch area; shows peek text (temp + "Tap for forecast") when collapsed. Heights: 90/40%/75% of screen. ScrollView scrollEnabled only in full state. ~75 tokens.
- `src/components/forecast/DailyForecast.tsx` — 7-day daily forecast rows. Shows day name, weather icon, min/max temps with bar, precip sum. Uses getWeatherInfo for codes. ~60 tokens.
- `src/components/alerts/AlertBanner.tsx` — NWS alert banner. Reads worst-severity alert from useAlerts, colored by severity. Navigates to /alert/[id] on press. ~45 tokens.
- `src/components/layers/LayerPicker.tsx` — FAB stack (right side, top:100). Shows only radar layer for rainviewer, all 5 layers for self-hosted. Fill layers use setActiveLayer (mutually exclusive); non-fill use toggleOverlay. Active state highlighted in #4fc3f7. ~70 tokens.
- `src/components/map/WeatherLayerOverlay.tsx` — Generic RasterSource/RasterLayer for self-hosted non-radar layers (temperature, wind, cape, precip-type). Uses buildSelfHostedTileUrl + LAYERS config for zoom bounds. ~50 tokens.
- `src/components/map/AlertPolygon.tsx` — MapLibre ShapeSource rendering NWS alert polygons. FillLayer + LineLayer colored by severity (Extreme/Severe/Moderate/Minor). Filters out alerts without geometry. ~60 tokens.

## Source: Types

- `src/types/weather.ts` — TypeScript types: RadarFrame, RainViewerManifest, OpenMeteoResponse, NWSAlert, TemperatureUnit, MapStyle, SelfHostedManifest, LayerType, DataSource, LayerConfig. ~100 tokens.

## Backend Pipeline (services/)

- `services/shared/color_tables.json` — RGBA color range definitions for reflectivity, temperature, wind_speed, cape, precip_type layers. ~120 tokens.
- `services/shared/tiler.py` — Shared tile renderer: apply_color_table, apply_categorical_color_table, render_tiles (numpy RGBA → XYZ PNG tiles). ~200 tokens.
- `services/shared/test_tiler.py` — pytest tests for tiler: test_apply_color_table, test_render_tiles_creates_files. ~80 tokens.
- `services/ingest-mrms/ingest.py` — MRMS radar ingest loop: polls NOAA S3, decodes GRIB2 via pygrib, renders reflectivity tiles every 2min. ~200 tokens.
- `services/ingest-mrms/requirements.txt` — numpy, Pillow, pygrib, httpx. ~10 tokens.
- `services/ingest-mrms/Dockerfile` — python:3.12-slim + libeccodes-dev. Build context: services/ dir. ~30 tokens.
- `services/ingest-hrrr/ingest.py` — HRRR forecast ingest: downloads per-hour GRIB2, extracts radar/temp/cape/wind/precip-type, renders tiles. ~300 tokens.
- `services/ingest-hrrr/requirements.txt` — numpy, Pillow, pygrib, httpx. ~10 tokens.
- `services/ingest-hrrr/Dockerfile` — python:3.12-slim + libeccodes-dev. Build context: services/ dir. ~30 tokens.
- `services/tile-server/Caddyfile` — Caddy :8080, /tiles/* static + /api/* reverse_proxy to FastAPI :8000, CORS headers. ~40 tokens.
- `services/tile-server/Dockerfile` — Multi-stage: python:3.12-slim API + caddy:2-alpine, /start.sh runs both. ~50 tokens.
- `services/tile-server/api/server.py` — FastAPI: GET /api/manifest.json (scan tile dirs), GET /api/forecast/{lat}/{lon} (Open-Meteo proxy w/ 15min cache), GET /api/health. ~150 tokens.
- `services/tile-server/api/requirements.txt` — fastapi, uvicorn, httpx. ~10 tokens.
- `services/tile-cleanup/cleanup.sh` — bash: find+delete radar dirs >4h, HRRR layers >8h. Runs in alpine container. ~20 tokens.
- `deploy/docker-compose.yml` — Orchestrates ingest-mrms, ingest-hrrr, tile-server, tile-cleanup with shared `tiles` volume. ~60 tokens.

## Tests

- `__tests__/lib/api.test.ts` — API function tests. ~60 tokens.
- `__tests__/lib/tileUrl.test.ts` — Tile URL builder tests. ~40 tokens.
- `__tests__/lib/weatherCodes.test.ts` — Weather code tests. ~30 tokens.
- `__tests__/stores/useWeatherStore.test.ts` — Zustand store tests. ~60 tokens.
