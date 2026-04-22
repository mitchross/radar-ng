# Project Anatomy — radar-ng

Expo SDK 55 React Native weather radar app (StormScope). Uses expo-router for file-based routing, Zustand for state, React Query for data fetching, MapLibre for map rendering.

UI design system: **Cumulus** (2026-04-17) — 3 tabs Home/Nowcast/Radar, violet accent #8B7CFF, condition-driven dark gradient backgrounds. Ported from UI-Handoff/design_handoff_cumulus_weather. Settings is now a modal, reachable via gear icon on Home top-right (preserves self-hosted data source toggle).

## Configuration

- `package.json` — dependencies: expo ~55, @tanstack/react-query ^5, expo-location, expo-router, zustand ^5, react-native-gesture-handler, maplibre-react-native. ~60 tokens.
- `tsconfig.json` — TypeScript config. ~20 tokens.
- `jest.config.js` — ts-jest preset, testEnvironment node, roots: __tests__, moduleNameMapper @/ → src/. ~30 tokens.
- `app.json` — Expo app config. ~40 tokens.

## Source: App Routes (expo-router)

- `src/app/_layout.tsx` — Root layout. Sets up GestureHandlerRootView, QueryClientProvider (retry:2, gcTime:10min), StatusBar light, Stack with (tabs) and alert/[id] (modal). ~50 tokens.
- `src/app/(tabs)/_layout.tsx` — Cumulus 3-tab layout (Home/Nowcast/Radar). Floating translucent rounded-pill bar (rgba 10,10,20,0.85, borderRadius 22, bottom 18). View-based icons (HomeIcon/NowcastIcon/RadarIcon), violet active tint. ~80 tokens.
- `src/app/(tabs)/index.tsx` — Cumulus Home. Condition-driven 5-stop gradient, gear→/settings modal, hero temp (112px Thin, -5 letter-spacing) with 140px WeatherIcon, location row w/ violet dot, nowcast banner (from minutely_15), 24h hourly strip, 24h precip chart card, 7-day list w/ temp-range bars + current-temp dot on Today, radar mini tease → /radar, stat grid (UV/Wind/Humidity/Visibility/Pressure/Dewpoint), Sun & Daylight arc. ~550 tokens.
- `src/app/(tabs)/nowcast.tsx` — thin re-export of screens/NowcastScreen. ~5 tokens.
- `src/app/(tabs)/radar.tsx` — Apple-Weather polish. WeatherMap full-bleed + RadarOverlay / WeatherLayerOverlay / AlertPolygon / LayerLocationMarker / EyedropperPin (long-press). LayerLegendCard top-left, RadarFABs right rail (layers/inspect-clear/style-picker), MapStylePicker modal (theme + projection), TimelineBar bottom ("Layer Name / Date" header). Alert banner top. ~130 tokens.
- `src/app/settings.tsx` — Settings modal (stack presentation="modal"). Close X top-left, clearNight gradient bg. Sections: Map (dark toggle), Units (F/C), Radar (opacity/playback sliders), Data Source (Free/Self-Hosted segmented + server URL TextInput + help hint). ~200 tokens.
- `src/app/alert/[id].tsx` — Alert detail modal screen. Uses useLocalSearchParams to get id. ~30 tokens.

## Source: Screens

- `src/screens/NowcastScreen.tsx` — Cumulus Nowcast. Back-chev header (HYPER-LOCAL NOWCAST kicker + location), hero verdict ("Rain starts in N min" w/ violet rain start count), 60-bar minute chart (interpolated from minutely_15, intensity-colored green→rain→violet→hot), chart axis + intensity scale gradient, KEY MOMENTS 2×2 grid (STARTS/PEAK/ENDS/TOTAL), FORECAST MODEL card (HRRR+MRMS blend, confidence bar), "About this forecast" note. ~300 tokens.

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
- `src/lib/weatherTheme.ts` — (legacy) CARROT Premium theme system — still used by older code paths. May be deleted once Cumulus fully replaces it. ~350 tokens.
- `src/lib/inspector.ts` — Inspector client. inspectPoint() hits /api/inspect on self-hosted; falls back to Open-Meteo current temp/wind for free tier; returns `{ok, value, unit, source}`. formatReading() layer-specific formatters (dBZ + intensity label, °F, mph, J/kg). ~110 tokens.
- `src/lib/cumulusTheme.ts` — Cumulus design tokens (violet accent, ink scale, cards, rain/snow/sun/temp/cold/hot/alert/ok). 7 CONDITION_GRADIENTS (clearDay/clearNight/cloudy/rain/storm/snow/fog, each 5-stop). getCumulusCondition(code,isNight), getIconKind(code,isNight)→IconKind, DBZ_SCALE (8-stop), getUVInfo / getWindInfo / getWindDirection / isNightAt. ~200 tokens.

## Source: Components — Weather

- `src/components/weather/WeatherScene.tsx` — (legacy, unused) CARROT weather scene illustration. Kept for reference. ~400 tokens.
- `src/components/weather/WeatherIcon.tsx` — Cumulus View-based weather icons (no SVG dep). Kinds: sun, moon, partlyCloudy, cloudy, overcast, rain, heavyRain, storm, snow, fog, hail. Scales linearly with size prop; 64-unit design grid. ~300 tokens.

## Source: Stores

- `src/stores/useWeatherStore.ts` — Zustand store. Default mapStyle:"dark", radarOpacity:0.8. State: frames, currentFrameIndex, isPlaying, lat/lon, radarOpacity/Visible, activeLayer, visibleOverlays, temperatureUnit, mapStyle, dataSource, serverUrl. Persists dataSource+serverUrl to MMKV via storage.ts. ~120 tokens.

## Source: Components

- `src/components/map/WeatherMap.tsx` — MapLibre MapView wrapper. Reads mapStyle/lat/lon/dataSource/serverUrl from Zustand, calls resolveMapStyleUrl() to pick public OpenFreeMap or self-hosted Protomaps URL. Renders Camera + UserLocation. ~60 tokens.
- `src/components/map/RadarOverlay.tsx` — Radar tile overlay. IEM NEXRAD tiles for free tier (tms=true, zoom 1-12), self-hosted tiles for selfhosted mode. No manifest dependency for free tier. ~75 tokens.
- `src/components/timeline/TimeSlider.tsx` — (Legacy) Radar timeline slider. Compact dark style. ~60 tokens.
- `src/components/timeline/PlayButton.tsx` — (Legacy) Play/pause button. ~50 tokens.
- `src/components/timeline/TimelineBar.tsx` — Cumulus segmented timeline. Dark glass card (bottom 92), violet play button, 4-segment track (past solid white / nowcast solid violet / HRRR dashed violet / long-range fainter dashed) computed from frame Unix seconds vs "now" using NOWCAST_MIN=60, HRRR_MIN=360. Green NOW marker, white thumb via Slider overlay with transparent min/max tracks. Axis: -1h/NOW/+1h/+6h/+24h. ~240 tokens.
- ~~`src/components/forecast/CurrentConditions.tsx`~~ — DELETED (forecast inlined into index.tsx).
- ~~`src/components/forecast/HourlyScroll.tsx`~~ — DELETED (forecast inlined into index.tsx).
- ~~`src/components/forecast/ForecastSheet.tsx`~~ — DELETED (forecast is now its own tab).
- ~~`src/components/forecast/DailyForecast.tsx`~~ — DELETED (forecast inlined into index.tsx).
- `src/components/alerts/AlertBanner.tsx` — NWS alert banner. paddingTop:44 (tighter to status bar). Reads worst-severity alert from useAlerts, colored by severity. Navigates to /alert/[id] on press. ~45 tokens.
- `src/components/layers/LayerPicker.tsx` — (Legacy) Dark-theme FAB stack for layer selection. ~65 tokens.
- `src/components/map/RadarFABs.tsx` — Cumulus right-side radar controls. Dark glass buttons (40x40, radius 12): layer picker (opens 200px popover listing Reflectivity/HRRR/Temp/Wind/Precip/CAPE — self-hosted-only layers gated on dataSource), pinpoint/inspector toggle (prop-driven), mapStyle toggle. Receives inspectorActive + onToggleInspector from parent. ~200 tokens.
- `src/components/map/WeatherLayerOverlay.tsx` — Generic RasterSource/RasterLayer for self-hosted non-radar layers (temperature, wind, cape, precip-type). Uses buildSelfHostedTileUrl + LAYERS config for zoom bounds. ~50 tokens.
- `src/components/map/AlertPolygon.tsx` — MapLibre ShapeSource rendering NWS alert polygons. FillLayer + LineLayer colored by severity (Extreme/Severe/Moderate/Minor). Filters out alerts without geometry. ~60 tokens.
- `src/components/map/LayerLocationMarker.tsx` — Apple-Weather-style My Location pill at user lat/lon. Layer-aware body (wind: dir/mph stacked; temperature/default: big temp number; cape: placeholder). MarkerView anchored bottom with tail + user dot + "My Location" caption. ~150 tokens.
- `src/components/map/LayerLegendCard.tsx` — Top-left vertical legend card. Per-layer LEGENDS map provides title + ordered stops for radar/radar-hrrr (dBZ), temperature (°F -40→130), wind (mph 0→75), precip-type (Light→Extreme), cape (J/kg 0→4000). Vertical LinearGradient + labels. "Map Data" attribution below. ~160 tokens.
- `src/components/map/MapStylePicker.tsx` — Bottom-sheet modal with flat/globe segmented toggle + 3 style tiles (Light/Dark/Satellite). Writes to mapStyle + mapProjection in store. Notes that Globe persists but RN MapLibre renders flat. ~120 tokens.
- `src/components/inspector/Eyedropper.tsx` — EyedropperPin: MarkerView at pinned lat/lon + readout panel top of screen. Fetches via inspectPoint() whenever pin/layer/frame changes, formats via formatReading(). Source label = Grid / Forecast / N/A. ~160 tokens.
- `src/components/palette/PaletteSelector.tsx` — 3-tile swatch picker (Classic NWS / Vivid CARROT-style / Muted viridis) for Settings. Writes activePalette to store. ~80 tokens.
- `src/components/timeline/CurrentForecastToggle.tsx` — Segmented pill above TimelineBar; writes store.timelineMode ("current" | "forecast"). Forecast mode merges past MRMS + nowcast + HRRR via useManifest. Hidden hint in free-tier ("Self-host to unlock forecast frames"). ~80 tokens.
- `src/components/map/LightningOverlay.tsx` — Reads useLightning(), renders ShapeSource with age-interpolated circle styles. Fresh strikes (< 60s) get a blurred halo; buffer fades 1.0 → 0.25 opacity over 15min retention. ~100 tokens.
- `src/components/map/TropicalOverlay.tsx` — NHC active storms: cone (translucent red fill), forecast track (dashed red line), current position (red dot + white stroke). ShapeSource with `filter` keyed on properties.kind. ~60 tokens.
- `src/hooks/useLightning.ts` — React Query hook fetching /api/lightning every 10s, gated on dataSource=selfhosted. ~25 tokens.
- `src/hooks/useTropical.ts` — React Query hook fetching /api/tropical every 5min, gated on dataSource=selfhosted. ~30 tokens.
- `src/hooks/useWindField.ts` — React Query hook fetching /api/wind-field/{timestamp}, gated on selfhosted+wind-like layer. Exports sampleWindField worklet (bilinear interp of int8-scaled U/V back to mph). ~90 tokens.
- `src/hooks/useStormCells.ts` — React Query hook fetching /api/storms (MRMS connected-component centroids) every 60s. ~30 tokens.
- `src/components/map/WindParticlesOverlay.tsx` — Skia Canvas overlay with 1200 advecting particles. UI-thread physics via useFrameCallback, compound Skia Paths per speed bin for one-drawcall-per-bin. Local web-mercator projection tied to SharedCamera (lon/lat/zoom shared values) from MapLibre onRegionIsChanging. ~300 tokens.
- `src/components/map/StormCellsOverlay.tsx` — ShapeSource with CircleLayer halo + core; radius interpolated from area_km2, color interpolated from peak_dbz (orange→magenta). ~70 tokens.

## Source: Types

- `src/types/weather.ts` — TypeScript types: RadarFrame, RainViewerManifest, OpenMeteoResponse, NWSAlert, TemperatureUnit, MapStyle, SelfHostedManifest, LayerType, DataSource, LayerConfig. ~100 tokens.

## Backend Pipeline (services/)

- `services/base/Dockerfile` — Shared stormscope-base:latest. python:3.12-slim + libeccodes-dev + eccodes-tools + gdal-bin + libgdal-dev. Installs numpy/Pillow/pygrib/httpx/mercantile. All ingestors FROM this. Must be built first: `docker compose --profile build-only build base`. ~40 tokens.
- `services/base/requirements.txt` — base-image pip deps (numpy, Pillow, pygrib, httpx, mercantile). ~10 tokens.
- `services/base/README.md` — build instructions + child-image pattern. ~30 tokens.
- `services/shared/color_tables.json` — (legacy) NWS palette, kept for backward compat. Superseded by services/shared/palettes/*.json. ~120 tokens.
- `services/shared/palettes/classic.json` — NWS-standard palette (green/yellow/red/magenta). Same structure as the legacy color_tables.json. ~120 tokens.
- `services/shared/palettes/vivid.json` — CARROT-style high-contrast palette (blue/cyan/pink/purple). ~120 tokens.
- `services/shared/palettes/muted.json` — viridis-based colorblind-safe palette. ~120 tokens.
- `services/shared/palettes.py` — Palette loader: get_palette_names() reads PALETTES env var (comma-separated, default "classic"); load_palette(name) reads services/shared/palettes/{name}.json, falls back to legacy color_tables.json for "classic". ~40 tokens.
- `services/shared/grid_dump.py` — write_grid(layer, timestamp, data, lats, lons, unit) dumps downsampled Float32 grid (capped at ~900x900 via stride-based downsampling) to /data/grids/{layer}/{timestamp}.bin + .meta.json. cleanup_old_grids() prunes > GRID_MAX_AGE_S (default 12h). Consumed by /api/inspect + /api/wind-field. Also handles 0..360 → -180..180 lon normalization. ~100 tokens.
- `services/shared/storms.py` — detect_storms() thresholds reflectivity ≥ STORM_THRESHOLD_DBZ (default 40), labels via scipy.ndimage.label, returns GeoJSON FeatureCollection of centroids with peak_dbz + area_km2 + pixel_count. Cap via STORM_MAX_CELLS (500). write_storms_json() persists to /data/state/storms.json for /api/storms. ~110 tokens.
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
- `services/tile-server/api/server.py` — FastAPI. GET /api/manifest.json (palette-aware layers, `latest` + `palettes` per layer), GET /api/forecast/{lat}/{lon} (Open-Meteo proxy, 15min cache), GET /api/inspect/{layer}/{timestamp}/{lat}/{lon} (bilinear sample from /data/grids Float32 dumps), GET /api/lightning (serves /data/state/lightning.json), GET /api/tropical (serves /data/state/tropical.json), GET /api/basemap/style/{name} (rewrites relative tiles to absolute), GET /api/health (503 degraded when MRMS stale), GET /api/metrics (Prometheus). ~300 tokens.
- `services/tile-server/api/requirements.txt` — fastapi, uvicorn, httpx. ~10 tokens.
- `services/tile-cleanup/cleanup.sh` — POSIX sh: radar >4h, nowcast >1h, HRRR forecast layers >12h. Handles both legacy `/tiles/{layer}/{timestamp}/` and multi-palette `/tiles/{layer}/{palette}/{timestamp}/` layouts. ~40 tokens.
- `services/nowcast/nowcast.py` — pysteps S-PROG nowcast. Reads last N MRMS Float32 grids from /data/grids/radar, runs DenseLucasKanade optical flow + S-PROG AR(2) forecast for HORIZON_MIN minutes at STEP_MIN cadence, renders per-palette tiles to /data/tiles/nowcast/{palette}/{timestamp}/. Persistence fallback when pysteps fails. ~180 tokens.
- `services/nowcast/Dockerfile` — FROM stormscope-base + gcc (purged after build) + pysteps 1.14, scipy, opencv-python-headless. ~30 tokens.
- `services/ingest-lightning/ingest.py` — asyncio WebSocket consumer of Blitzortung (wss://443 preferred, 8087-8090 fallback). LZW decode with fallback to raw-JSON. Maintains deque of last 15min strikes, writes compact GeoJSON to /data/state/lightning.json every 2s. Regional bbox filter (default CONUS + Atlantic). ~180 tokens.
- `services/ingest-lightning/Dockerfile` — FROM stormscope-base + websockets==12.0. ~15 tokens.
- `services/ingest-tropical/ingest.py` — httpx fetch of NHC CurrentStorms.json every 5min. Builds a merged FeatureCollection with one feature per storm (position point + forecast track line + cone polygon). Writes /data/state/tropical.json. ~110 tokens.
- `services/ingest-tropical/Dockerfile` — FROM stormscope-base (httpx already present). ~10 tokens.
- `services/base/requirements.txt` — base-image pip deps: numpy, Pillow, pygrib, httpx, mercantile, scipy (for nowcast + storms.py). ~10 tokens.
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
