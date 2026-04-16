# Project Anatomy — radar-ng

Expo SDK 55 React Native weather radar app (StormScope). Uses expo-router for file-based routing, Zustand for state, React Query for data fetching, MapLibre for map rendering.

## Configuration

- `package.json` — dependencies: expo ~55, @tanstack/react-query ^5, expo-location, expo-router, zustand ^5, react-native-gesture-handler, maplibre-react-native. ~60 tokens.
- `tsconfig.json` — TypeScript config. ~20 tokens.
- `jest.config.js` — ts-jest preset, testEnvironment node, roots: __tests__, moduleNameMapper @/ → src/. ~30 tokens.
- `app.json` — Expo app config. ~40 tokens.

## Source: App Routes (expo-router)

- `src/app/_layout.tsx` — Root layout. Sets up GestureHandlerRootView, QueryClientProvider (retry:2, gcTime:10min), StatusBar light, Stack with (tabs) and alert/[id] (modal). ~50 tokens.
- `src/app/(tabs)/_layout.tsx` — 3-tab layout (Weather/Radar/Settings). Semi-transparent tab bar (rgba 0d1117 0.95), custom View-based icons (SunIcon/RadarIcon/GearIcon — no emoji), accent #42A5F5, hairline border. ~80 tokens.
- `src/app/(tabs)/index.tsx` — CARROT Premium forecast screen. 5-stop LinearGradient background (weather-adaptive), WeatherScene illustration header (city skyline + celestial objects), hero temp (108px, adaptive weight), H/L/feels-like inline, snarky quote card, alert card with dot indicator, hourly chart (scrollable, temp dots + connecting lines + precip%), 7-day daily (with precip% column + gradient temp bars), Wind & Pressure card (compass visual + stats), Sun & Moon card (sunrise/sunset/daylight + progress bar), Atmosphere card (UV gauge + humidity gauge + dewpoint + pressure), Precipitation Chance bar chart (12h). ~550 tokens.
- `src/app/(tabs)/radar.tsx` — CARROT-style radar map screen. Full-bleed WeatherMap (light map default) + overlays. Uses RadarFABs (CARROT-style white circle FABs), TimelineBar (new redesigned timeline), AlertBanner. ~60 tokens.
- `src/app/(tabs)/settings.tsx` — Settings screen with LinearGradient background (#0D1B2A→#263238). Glassmorphism cards (rgba white 0.08, borderRadius 20). Section/Row/SegmentedControl helpers. Accent #42A5F5. ~240 tokens.
- `src/app/alert/[id].tsx` — Alert detail modal screen. Uses useLocalSearchParams to get id. ~30 tokens.

## Source: Hooks

- `src/hooks/useManifest.ts` — Dual-source manifest hook. IEM frames built deterministically via buildIEMFrames() on interval (no API call for free tier). selfHostedQuery for self-hosted mode. Syncs frames to store. ~90 tokens.
- `src/hooks/useForecast.ts` — React Query hook for Open-Meteo forecast. Enabled when lat/lon set. refetchInterval: 15min. ~35 tokens.
- `src/hooks/useAlerts.ts` — React Query hook for NWS alerts. Enabled when lat/lon set. refetchInterval: 60s. ~35 tokens.
- `src/hooks/useLocation.ts` — expo-location hook. Requests foreground permission, falls back to DEFAULTS lat/lon on denial. Sets location in Zustand. ~45 tokens.

## Source: Lib

- `src/lib/api.ts` — fetchRadarManifest, buildIEMFrames (deterministic IEM NEXRAD frame list), fetchForecast(lat, lon, opts?: {dataSource, serverUrl}) — routes through `${serverUrl}/api/forecast/{lat}/{lon}` when dataSource=selfhosted, else public Open-Meteo, fetchAlerts, fetchSelfHostedManifest, checkServerHealth. ~150 tokens.
- `src/lib/constants.ts` — API URLs, MAP_STYLES_PUBLIC (OpenFreeMap), MAP_STYLES_SELFHOSTED (/basemap/styles/*), resolveMapStyleUrl(dataSource, serverUrl, mapStyle) helper, MAP_STYLES legacy alias, RADAR config, IEM config, DEFAULTS, SELF_HOSTED (FORECAST_PATH, HEALTH_PATH, METRICS_PATH, BASEMAP_TILE_PATTERN), LAYERS. ~110 tokens.
- `src/lib/storage.ts` — MMKV wrapper using createMMKV({id:"stormscope"}). getString, setString, getBoolean, setBoolean helpers. ~40 tokens.
- `src/lib/tileUrl.ts` — buildRadarTileUrl (RainViewer), buildIEMTileUrl (IEM NEXRAD), buildSelfHostedTileUrl (self-hosted). ~60 tokens.
- `src/lib/weatherCodes.ts` — WMO weather code descriptions. ~60 tokens.
- `src/lib/weatherTheme.ts` — CARROT Premium weather theme system: 5-stop gradient backgrounds (11 weather categories × day/night), SceneType for illustrations (sunny/night_clear/cloudy/rainy/snowy/thunderstorm/etc), scene colors (skyline/celestial/particles), getTempFontWeight, getTempColor, getWindInfo (speed→label+color), getUVInfo (index→label+color), getWindDirection (degrees→compass), snarky personality quotes. ~350 tokens.

## Source: Components — Weather

- `src/components/weather/WeatherScene.tsx` — CARROT Premium-style weather scene illustration. View-based city skyline silhouette (16 buildings with window lights), celestial objects (sun with rays/glow, moon with crescent, stars), weather effects (clouds, rain drops, snow flakes, lightning bolt). Adapts to SceneType from theme. All React Native Views, no SVG dependency. ~400 tokens.

## Source: Stores

- `src/stores/useWeatherStore.ts` — Zustand store. Default mapStyle:"dark", radarOpacity:0.8. State: frames, currentFrameIndex, isPlaying, lat/lon, radarOpacity/Visible, activeLayer, visibleOverlays, temperatureUnit, mapStyle, dataSource, serverUrl. Persists dataSource+serverUrl to MMKV via storage.ts. ~120 tokens.

## Source: Components

- `src/components/map/WeatherMap.tsx` — MapLibre MapView wrapper. Reads mapStyle/lat/lon/dataSource/serverUrl from Zustand, calls resolveMapStyleUrl() to pick public OpenFreeMap or self-hosted Protomaps URL. Renders Camera + UserLocation. ~60 tokens.
- `src/components/map/RadarOverlay.tsx` — Radar tile overlay. IEM NEXRAD tiles for free tier (tms=true, zoom 1-12), self-hosted tiles for selfhosted mode. No manifest dependency for free tier. ~75 tokens.
- `src/components/timeline/TimeSlider.tsx` — (Legacy) Radar timeline slider. Compact dark style. ~60 tokens.
- `src/components/timeline/PlayButton.tsx` — (Legacy) Play/pause button. ~50 tokens.
- `src/components/timeline/TimelineBar.tsx` — CARROT-style unified timeline bar. White bg, blue play button (38x38), blue slider, current time display + LIVE badge, timestamp ticks (start/middle/end) below slider. Includes own play/pause logic + interval. ~120 tokens.
- ~~`src/components/forecast/CurrentConditions.tsx`~~ — DELETED (forecast inlined into index.tsx).
- ~~`src/components/forecast/HourlyScroll.tsx`~~ — DELETED (forecast inlined into index.tsx).
- ~~`src/components/forecast/ForecastSheet.tsx`~~ — DELETED (forecast is now its own tab).
- ~~`src/components/forecast/DailyForecast.tsx`~~ — DELETED (forecast inlined into index.tsx).
- `src/components/alerts/AlertBanner.tsx` — NWS alert banner. paddingTop:44 (tighter to status bar). Reads worst-severity alert from useAlerts, colored by severity. Navigates to /alert/[id] on press. ~45 tokens.
- `src/components/layers/LayerPicker.tsx` — (Legacy) Dark-theme FAB stack for layer selection. ~65 tokens.
- `src/components/map/RadarFABs.tsx` — CARROT-style floating action buttons for radar screen. White circle FABs (44x44) with drop shadow: LocationArrow (blue arrow icon), Layers (3 stacked lines, white when active on blue bg), MapStyle toggle (map icon, toggles light/dark). ~100 tokens.
- `src/components/map/WeatherLayerOverlay.tsx` — Generic RasterSource/RasterLayer for self-hosted non-radar layers (temperature, wind, cape, precip-type). Uses buildSelfHostedTileUrl + LAYERS config for zoom bounds. ~50 tokens.
- `src/components/map/AlertPolygon.tsx` — MapLibre ShapeSource rendering NWS alert polygons. FillLayer + LineLayer colored by severity (Extreme/Severe/Moderate/Minor). Filters out alerts without geometry. ~60 tokens.

## Source: Types

- `src/types/weather.ts` — TypeScript types: RadarFrame, RainViewerManifest, OpenMeteoResponse, NWSAlert, TemperatureUnit, MapStyle, SelfHostedManifest, LayerType, DataSource, LayerConfig. ~100 tokens.

## Backend Pipeline (services/)

- `services/base/Dockerfile` — Shared stormscope-base:latest. python:3.12-slim + libeccodes-dev + eccodes-tools + gdal-bin + libgdal-dev. Installs numpy/Pillow/pygrib/httpx/mercantile. All ingestors FROM this. Must be built first: `docker compose --profile build-only build base`. ~40 tokens.
- `services/base/requirements.txt` — base-image pip deps (numpy, Pillow, pygrib, httpx, mercantile). ~10 tokens.
- `services/base/README.md` — build instructions + child-image pattern. ~30 tokens.
- `services/shared/color_tables.json` — RGBA color range definitions for reflectivity, temperature, wind_speed, cape, precip_type layers. Phase-4 prep: optional dewpoint/humidity entries consumed by ingest-hrrr if present. ~120 tokens.
- `services/shared/tiler.py` — Shared tile renderer: apply_color_table, apply_categorical_color_table, render_tiles (numpy RGBA → XYZ PNG tiles). ~200 tokens.
- `services/shared/test_tiler.py` — pytest tests for tiler: test_apply_color_table, test_render_tiles_creates_files. ~80 tokens.
- `services/shared/logger.py` — JsonFormatter + get_logger + retry(attempts, base_delay, max_delay, exceptions) decorator. Used by every ingestor for structured logs + exponential-backoff retries. ~80 tokens.
- `services/shared/state.py` — ProcessedSet(path, max_entries): bounded on-disk set of processed-ids with atomic tempfile writes. Survives container restart so we don't re-render already-processed S3 objects. ~60 tokens.
- `services/ingest-mrms/ingest.py` — MRMS radar ingest loop (HARDENED Phase 1.4). Polls NOAA S3 with retry+backoff, decodes GRIB2 via pygrib, renders reflectivity tiles every 2min. JSON logs. State persisted to /data/state/ingest-mrms.json. ~280 tokens.
- `services/ingest-mrms/requirements.txt` — unused (deps in base image); kept for local dev reference. ~5 tokens.
- `services/ingest-mrms/Dockerfile` — FROM stormscope-base:latest. Copies shared/ + ingest.py. STATE_DIR=/data/state. ~20 tokens.
- `services/ingest-hrrr/ingest.py` — HRRR forecast ingest (HARDENED Phase 1.4). Byte-range subsetting via .idx sidecar (downloads only records we render). JSON logs. Retry+backoff. State persisted. FORECAST_HOURS=18 default, EXTENDED_FORECAST_HOURS=48 for 00/06/12/18z runs. Renders radar-hrrr + temperature + dewpoint + humidity + wind + cape + precip-type layers. ~500 tokens.
- `services/ingest-hrrr/requirements.txt` — unused (deps in base image). ~5 tokens.
- `services/ingest-hrrr/Dockerfile` — FROM stormscope-base:latest. STATE_DIR=/data/state, FORECAST_HOURS=18, EXTENDED_FORECAST_HOURS=48. ~25 tokens.
- `services/tile-server/Caddyfile` — Caddy :8080. /tiles/* static file_server, /basemap/tiles/* → basemap:8081 reverse_proxy, /basemap/styles/* static from /srv/basemap, /api/* → FastAPI :8000. CORS + cache headers per route. ~60 tokens.
- `services/tile-server/Dockerfile` — python:3.12-slim + Caddy binary. Copies api/, Caddyfile, basemap/styles/ into /srv/basemap. /start.sh runs uvicorn + caddy. OPEN_METEO_BASE env. ~50 tokens.
- `services/tile-server/api/server.py` — FastAPI (HARDENED Phase 1.4). GET /api/manifest.json (includes `latest` per layer), GET /api/forecast/{lat}/{lon} (proxies OPEN_METEO_BASE with 15min cache; returns 502 on upstream error; extended fields dew_point_2m/surface_pressure/uv_index_max/precipitation_probability_max), GET /api/health (returns 503 degraded when MRMS tiles stale > MRMS_MAX_AGE_S), GET /api/metrics (Prometheus text format: request counters, per-layer timestamp gauges, mrms_age_seconds). ~220 tokens.
- `services/tile-server/api/requirements.txt` — fastapi, uvicorn, httpx. ~10 tokens.
- `services/tile-cleanup/cleanup.sh` — POSIX sh: radar >4h, nowcast >1h, HRRR forecast layers >12h. Covers radar-hrrr + temperature + dewpoint + humidity + wind + cape + precip-type. ~30 tokens.
- `services/basemap/README.md` — Protomaps setup: drop regional .pmtiles into ../pmtiles-data/basemap.pmtiles. ~50 tokens.
- `services/basemap/styles/positron.json` — light MapLibre style (Protomaps vector sources). Loaded by app when dataSource=selfhosted + mapStyle=light. ~350 tokens.
- `services/basemap/styles/dark-matter.json` — dark MapLibre style. Accent highway color #42A5F5 matches app accent. ~350 tokens.
- `deploy/docker-compose.yml` — Orchestrates base (build-only profile), ingest-mrms, ingest-hrrr, open-meteo, open-meteo-sync (profile:sync), basemap, tile-server, tile-cleanup. Shared volumes: tiles, state, open-meteo-data. Default json-file logging with 10m/3-file rotation. ~200 tokens.

## Docs

- `docs/PLAN.md` — StormScope v3.0 master implementation plan (7 phases). Phase 1: Self-hosting foundation (base Docker image, self-hosted Open-Meteo, Protomaps base map, backend hardening). Phase 2: Complete radar timeline (nowcast-pysteps, 48h HRRR, Current/Forecast toggle, blend). Phase 3: Inspector tool (eyedropper), 3 color palettes, 6 map themes + globe. Phase 4: 6 map types (temp, rain totals, snow, humidity, wind particles via Skia). Phase 5: NEXRAD L3 per-station, lightning, storm cells, warnings sub-layers. Phase 6: Cloud cover, tropical, fronts, widgets, notifications, offline. Phase 7: K8s deployment (Talos/ArgoCD). Dependency graph, resource requirements (~32 CPU, ~36GB RAM, ~650GB disk). ~1200 tokens.

## Tests

- `__tests__/lib/api.test.ts` — API function tests. ~60 tokens.
- `__tests__/lib/tileUrl.test.ts` — Tile URL builder tests. ~40 tokens.
- `__tests__/lib/weatherCodes.test.ts` — Weather code tests. ~30 tokens.
- `__tests__/stores/useWeatherStore.test.ts` — Zustand store tests. ~60 tokens.
