# StormScope v3.0 — Implementation Plan

> **⚠️ Historical document.** This plan was written before the 2026-05-02 Temporal refactor and refers to paths that no longer exist (`services/...` is now `backend/...`, `src/...` is now `frontend/src/...`, all CronJobs are now Temporal Schedules). Most of Phases 1–7 below have shipped. Treat this as a record of how the app got built, not as a current task list. For the current repo layout, see the README "Project layout" section. For the Temporal architecture, see `docs/superpowers/specs/2026-04-30-temporal-radar-ng-design.md`.

## Context

StormScope is a self-hosted weather radar app (React Native/Expo) with a containerized backend pipeline. The current app has 3 tabs (Weather forecast, Radar map, Settings) with IEM NEXRAD free-tier radar and a Docker backend (ingest-mrms, ingest-hrrr, tile-server, tile-cleanup) that's built but not battle-tested. The Weather tab was recently redesigned to CARROT Premium style (gradient backgrounds, city skyline illustrations, glassmorphism cards). The Radar tab was redesigned with light map base and CARROT-style FABs.

**The problem:** The app still depends on external APIs (Open-Meteo for forecasts, OpenFreeMap for base map tiles), the radar only shows ~50 minutes of past data via IEM, there's no forecast radar, no nowcast, no inspector tool, no color palette options, and the UI lacks the depth of features CARROT Weather offers (individual stations, sub-layers, wind particles, etc.).

**The goal:** A FULLY self-hosted, ZERO-API-key weather app matching or exceeding CARROT Weather, running entirely on Mitch's Talos/ArgoCD homelab. PRD v3.0 defines 25+ ingest containers, 6 map types, 12+ sub-layers, inspector tool, animated wind particles, widgets, and more.

**This plan** breaks the PRD into 7 executable phases, each delivering a usable increment.

---

## Phase 1 — Self-Hosting Foundation (3-4 weekends)

**Goal:** Eliminate ALL external API dependencies. After this phase, the app runs 100% on your server.

### 1.1 Shared Base Docker Image
Create a single base image all Python ingestors extend from.

- **Create** `services/base/Dockerfile` — `python:3.12-slim` + `wgrib2` + `gdal-bin` + `libeccodes-dev` + `libgdal-dev` + common Python deps (`numpy`, `Pillow`, `pygrib`, `httpx`, `herbie-data`, `mercantile`, `pyart`)
- **Create** `services/base/requirements.txt`
- **Modify** `services/ingest-mrms/Dockerfile` → `FROM stormscope-base`
- **Modify** `services/ingest-hrrr/Dockerfile` → `FROM stormscope-base`
- **Modify** `deploy/docker-compose.yml` — add base image build

### 1.2 Self-Hosted Open-Meteo
Replace external `api.open-meteo.com` with the open-source Open-Meteo Swift service running locally.

- **Add** `open-meteo` service to `deploy/docker-compose.yml` using `ghcr.io/open-meteo/open-meteo` upstream image
- Configure to ingest GFS (global, 16-day) + HRRR (CONUS, 48h) directly from NOAA S3
- Mount persistent volume (~20GB for model cache)
- **Modify** `services/tile-server/api/server.py` — proxy forecasts to local Open-Meteo when `OPEN_METEO_BASE` env var points to local instance; add missing fields (`dew_point_2m`, `surface_pressure`, `uv_index_max`, `precipitation_probability_max`)
- **Modify** `src/lib/api.ts` — `fetchForecast()` routes through self-hosted server when `dataSource === "selfhosted"`
- **Modify** `src/hooks/useForecast.ts` — pass server URL

### 1.3 Self-Hosted Base Map (Protomaps)
Replace OpenFreeMap with locally-served vector tiles.

- Download Protomaps North America PMTiles (~35GB) onto server
- **Create** `services/basemap/` directory with config
- **Add** basemap service to `deploy/docker-compose.yml` using `protomaps/go-pmtiles` or `maptiler/tileserver-gl`
- **Create** `services/basemap/styles/positron.json` (light), `dark-matter.json` (dark)
- **Modify** `services/tile-server/Caddyfile` — add `/basemap/*` reverse-proxy
- **Modify** `src/lib/constants.ts` — `MAP_STYLES` uses self-hosted URLs when `dataSource === "selfhosted"`
- **Modify** `src/components/map/WeatherMap.tsx` — dynamic `mapStyle` URL

### 1.4 Backend Hardening
Make the existing pipeline production-ready.

- **Modify** `services/ingest-mrms/ingest.py` — add structured JSON logging, retry with exponential backoff on S3 downloads, persist processed-files list to disk (survives restart)
- **Modify** `services/ingest-hrrr/ingest.py` — same + switch to `herbie` for byte-range subsetting (download ~200MB per forecast hour instead of ~2.5GB full file)
- **Modify** `services/tile-server/api/server.py` — `/api/health` checks tile freshness (degraded if MRMS stale >10min), add `/api/metrics` endpoint

### Verification
- `curl localhost:8080/api/forecast/42.96/-85.67` returns data from LOCAL Open-Meteo
- Map renders with zero external network calls (disconnect internet, everything loads)
- Pipeline runs 24h without crashes
- HRRR downloads are ~200MB/hr (byte-range) not ~2.5GB/hr

---

## Phase 2 — Complete Radar Timeline: Nowcast + 48h Forecast (4-5 weekends)

**Goal:** Seamless timeline from -3h past through +48h future. This is the #1 UX gap vs CARROT.

**Depends on:** Phase 1 (base image, hardened pipeline, self-hosted forecast)

### 2.1 Extend HRRR to 48h
- **Modify** `services/ingest-hrrr/ingest.py` — detect extended runs (00z/06z/12z/18z), download f01-f48; standard runs stay at f18
- Use `herbie` for efficient byte-range subsetting
- Update retention: 12h for extended runs
- **Modify** `services/tile-server/api/server.py` — manifest includes `forecast_range` per layer

### 2.2 Build nowcast-pysteps Container
- **Create** `services/nowcast/nowcast.py` — load last 6-8 MRMS frames, run pysteps STEPS optical flow, generate 36 frames (5-min intervals, 0 to +3h)
- **Create** `services/nowcast/Dockerfile` (FROM stormscope-base + `pysteps`)
- **Create** `services/nowcast/requirements.txt`
- **Modify** `deploy/docker-compose.yml` — add nowcast service
- Output to `/data/tiles/nowcast/{timestamp}/{z}/{x}/{y}.png`
- Update manifest to include nowcast timestamps
- Resources: 2 CPU, 2GB RAM

### 2.3 Current/Forecast Toggle
- **Create** `src/components/timeline/CurrentForecastToggle.tsx` — segmented control ("Current" | "Forecast")
- **Modify** `src/stores/useWeatherStore.ts` — add `timelineMode: "current" | "forecast"`
- **Modify** `src/hooks/useManifest.ts` — merge MRMS past + nowcast + HRRR forecast into unified sorted timeline; filter by active mode
- **Modify** `src/app/(tabs)/radar.tsx` — add toggle above timeline
- **Modify** `src/components/timeline/TimelineBar.tsx` — confidence gradient on slider (solid=observed, fading=forecast), adjust ticks for 48h range

### 2.4 Nowcast→HRRR Blend
- At +2.5h to +3.5h, cross-fade between nowcast and HRRR tiles
- Implement via dual RasterSource in `RadarOverlay.tsx` with computed opacity

### Verification
- Timeline scrubs seamlessly: past MRMS → nowcast → HRRR forecast
- No visual "pop" at the nowcast/HRRR boundary
- Nowcast storm cells move in correct direction
- Slider covers -3h to +48h in Forecast mode

---

## Phase 3 — Inspector, Color Palettes, Map Styles (3-4 weekends)

**Goal:** Interactive features that differentiate from basic radar viewers.

**Depends on:** Phase 1 (base image, pipeline)

### 3.1 Inspector Tool (Eyedropper)
**Backend:**
- **Modify** `services/ingest-mrms/ingest.py` + `ingest-hrrr/ingest.py` — save downsampled Float32 grids alongside tiles to `/data/grids/{layer}/{timestamp}.bin` + `.meta.json`
- **Modify** `services/tile-server/api/server.py` — serve grid files at `/data/` prefix, add fallback `/api/inspect/{layer}/{timestamp}/{lat}/{lon}`

**App:**
- **Create** `src/lib/inspector.ts` — decode Float32Array, bilinear interpolation lat/lon lookup
- **Create** `src/components/inspector/Eyedropper.tsx` — crosshair at screen center, readout pill with value + label
- **Create** `src/components/inspector/InspectorProvider.tsx` — React context managing grid loading + caching (3 grids in memory)
- Layer-specific readout formatters (Radar: "42 dBZ Heavy Rain", Temp: "78°F", Wind: "25 mph NW")
- Long-press on map → one-shot inspect at that point
- **Modify** `src/app/(tabs)/radar.tsx` — wrap with InspectorProvider, add eyedropper FAB
- **Modify** `src/components/map/RadarFABs.tsx` — add eyedropper button

### 3.2 Three Color Palettes
**Backend:**
- **Create** `services/shared/palettes/classic.json` (NWS standard — existing colors)
- **Create** `services/shared/palettes/vivid.json` (CARROT-style blue/cyan/pink/purple)
- **Create** `services/shared/palettes/muted.json` (viridis-based, colorblind-safe)
- **Modify** `services/shared/tiler.py` — `render_tiles()` accepts palette name, outputs to `/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png`
- **Modify** ingest scripts — render 3x tile sets per frame
- **Modify** `services/tile-cleanup/cleanup.sh` — handle extra directory depth

**App:**
- **Create** `src/components/palette/PaletteSelector.tsx` — 3-swatch picker
- **Modify** `src/lib/tileUrl.ts` — include palette in self-hosted tile URLs
- **Modify** `src/stores/useWeatherStore.ts` — add `activePalette: "classic" | "vivid" | "muted"` with MMKV persistence
- **Modify** `src/app/(tabs)/settings.tsx` — palette preview section

### 3.3 Six Map Themes
- **Create** `src/components/globe/MapStylePicker.tsx` — 2x3 grid preview (light/dark/satellite × flat/globe)
- **Create** `src/components/globe/ProjectionToggle.tsx` — round/flat toggle
- **Modify** `src/stores/useWeatherStore.ts` — add `projection: "flat" | "globe"`, extend `MapStyle` to include `"satellite"`
- **Modify** `src/components/map/WeatherMap.tsx` — apply MapLibre `projection: 'globe'`
- **Modify** `src/components/map/RadarFABs.tsx` — replace simple toggle with MapStylePicker trigger

### Verification
- Inspector readout updates at 60fps while panning
- Grid download is <3MB gzipped
- Three distinct palettes are visually different
- Globe projection renders earth curvature at low zoom
- Muted palette passes colorblindness simulator check

---

## Phase 4 — Additional Map Types + Wind Particles (5-6 weekends)

**Goal:** Expand from 1 radar layer to all 6 map types.

**Depends on:** Phase 2 (48h HRRR), Phase 3 (palettes, inspector)

### 4.1 Map Type Architecture
- **Create** `src/types/mapTypes.ts` — defines 6 map types with sub-layers
- **Create** `src/components/map/MapTypeSelector.tsx` — horizontal scrollable pill bar
- **Modify** `src/stores/useWeatherStore.ts` — replace `activeLayer` with `mapType` + `enabledSubLayers`

### 4.2 Temperature Map
- Already extracted by `ingest-hrrr` — needs dewpoint (`DPT`) and feels-like (computed)
- **Modify** `services/ingest-hrrr/ingest.py` — add DPT extraction + derived feels-like field

### 4.3 Rain Totals Map (QPE Accumulation)
- **Create** `services/ingest-qpe/` — MRMS `RadarOnly_QPE_01H`, `_06H`, `_24H`, `_72H`
- Future accumulation from HRRR `APCP` summed over configurable windows
- **Create** `src/components/map/AccumulationOverlay.tsx` — with time-window pill selector

### 4.4 Snow Depth Map
- **Create** `services/ingest-snodas/` — NOHRSC SNODAS daily GeoTIFF
- Products: current depth, 24hr snowfall, SWE
- Simple daily ingest (~500MB download)

### 4.5 Humidity Map
- **Modify** `services/ingest-hrrr/ingest.py` — add RH extraction + color table

### 4.6 Wind Particles (Skia) — the marquee feature
- **Add** `@shopify/react-native-skia` to `package.json` (native rebuild required)
- **Create** `src/components/wind/WindParticleCanvas.tsx` — Skia canvas overlay, ~2000 particles with fading trails, velocity from U/V grid, color by wind speed
- **Create** `src/lib/windGrid.ts` — fetch/decode interleaved U/V Float32 binary
- **Modify** `services/ingest-hrrr/ingest.py` — save raw U/V grids as `/data/wind-uv/{timestamp}.bin`
- Performance target: 30fps on mid-range Android

### Verification
- All 6 map types render, scrub, and inspect correctly
- Wind particles flow in correct direction, speed up in high-wind areas
- Particles stay above 30fps
- Accumulation shows sensible values (check against NWS reported totals)

---

## Phase 5 — Individual Stations + Sub-Layers (4-5 weekends)

**Goal:** Per-station NEXRAD data + information-rich overlays.

**Depends on:** Phase 4 (map type architecture)

### 5.1 NEXRAD Level III Per-Station Ingest
- **Create** `services/ingest-nexrad-l3/` — pulls from `s3://unidata-nexrad-level3/SI.{station}/`
- Parse with `pyart`: N0Q, N0U, N1P (1hr accum = "Accumulation Array"), NTP (storm total), NST (storm tracking)
- Render per-station tiles at z6-z11
- **Create** `services/ingest-nexrad-l3/stations.json` — 160 CONUS station catalog
- Scale: shard across replicas via `STATIONS_SHARD=0/3` env var
- Resources: 2 CPU, 2GB RAM per replica

### 5.2 Station UI
- **Create** `src/components/stations/StationMarker.tsx` — blue tower icons on map
- **Create** `src/components/stations/StationDetailSheet.tsx` — bottom sheet with station name, circular radar image, product toggle, mini-timeline
- **Create** `src/components/stations/StationProductPicker.tsx` — N0Q/N0U/N1P/NTP toggle
- **Create** `src/hooks/useStations.ts` — fetches `/api/stations`
- **Create** `src/app/station/[id].tsx` — full station detail route

### 5.3 Lightning Strikes Sub-Layer
- **Create** `services/ingest-glm/` — GOES-16/18 GLM NetCDF → `/api/lightning.json` (rolling 60-min GeoJSON)
- **Create** `src/components/sublayers/LightningStrikes.tsx` — yellow icons fading over 10min, clustered at low zoom

### 5.4 Storm Cells Sub-Layer
- Extract NST storm attributes from NEXRAD L3 data
- **Create** `src/components/sublayers/StormCells.tsx` — circle markers with direction arrows, tap for details (VIL, hail %, TVS)

### 5.5 Warnings as Toggleable Sub-Layer
- Promote existing `AlertPolygon.tsx` from always-on to toggleable
- Add labels at polygon centroids, severity filtering

### Verification
- Tap station icon → bottom sheet loads in <300ms
- Station radar matches NWS radar viewer output
- Lightning strikes appear within 2 minutes of real GLM data
- Storm cell arrows point in correct movement direction

---

## Phase 6 — Advanced Overlays + Platform Integration (5-6 weekends)

**Goal:** Remaining sub-layers + widgets/notifications polish.

**Depends on:** Phase 5

### 6.1 Cloud Cover (GOES ABI)
- **Create** `services/ingest-goes-abi/` — visible (day) + IR (night), grayscale tiles, 5-min
- **Create** `src/components/sublayers/CloudCover.tsx`

### 6.2 Tropical Storms (NHC)
- **Create** `services/ingest-nhc/` — forecast cones, tracks, wind radii → `/api/tropical.json`
- **Create** `src/components/sublayers/TropicalStorms.tsx`

### 6.3 Pressure Fronts
- **Create** `services/ingest-fronts/` — WPC surface analysis → `/api/fronts.json` (cold/warm/stationary/occluded LineStrings)
- **Create** `src/components/sublayers/PressureFronts.tsx` — styled polylines

### 6.4 Home Screen Widgets
- **Modify** `services/tile-server/api/server.py` — `/api/widget/snapshot/{lat}/{lon}/{zoom}/{size}/{theme}.png`
- **Create** iOS WidgetKit extension (Swift) — small/medium/large
- **Create** Android Glance widget (Kotlin)

### 6.5 Background Notifications
- **Create** `src/services/notificationService.ts` — background fetch for severe weather + "rain starting soon"
- Add `expo-notifications` + `expo-task-manager`
- Notification toggles in settings

### 6.6 Offline Mode
- **Create** `src/lib/offlineCache.ts` — cache tiles + forecast on device
- **Create** `src/components/common/StalenessBadge.tsx`
- Add `@react-native-community/netinfo`

---

## Phase 7 — Kubernetes Deployment (2-3 weekends, parallelizable)

**Goal:** Move from Docker Compose to production K8s on Talos/ArgoCD.

**Can run in parallel with Phases 2-6** — pure infrastructure.

- **Create** `deploy/k8s/base/` — Kustomize manifests for all services
- Ingest containers → Deployments with appropriate replica counts
- Open-Meteo + basemap → StatefulSets (persistent storage)
- Tile-cleanup → CronJob (replaces while-true-sleep)
- Shared tiles volume → PVC (Longhorn-backed, VolSync to TrueNAS)
- **Create** `deploy/argocd/application.yaml` — ArgoCD Application pointing at repo
- **Create** monitoring: ServiceMonitor, Grafana dashboard, AlertManager rules
- Ingress via Cloudflare Tunnel → `stormscope.vanillax.me`

---

## Dependency Graph

```
Phase 1 (Self-Hosting Foundation)
  ├── Phase 2 (Nowcast + 48h + Toggle)
  │     └── Phase 4 (Map Types + Wind Particles)
  │           └── Phase 5 (Stations + Sub-Layers)
  │                 └── Phase 6 (Advanced + Polish)
  ├── Phase 3 (Inspector + Palettes + Styles)
  │     └── Phase 4
  └── Phase 7 (K8s) — parallelizable anytime
```

Phases 2 and 3 can be worked in parallel (different parts of codebase).

## Recommended Solo Dev Order
1. Phase 1 → 2. Phase 3 → 3. Phase 2 → 4. Phase 4 → 5. Phase 5 → 6. Phase 7 → 7. Phase 6

**Total: ~25-35 weekends (6-9 months evenings/weekends)**

---

## Critical Files (touch most often)

| File | Why |
|---|---|
| `deploy/docker-compose.yml` | Every phase adds services |
| `services/shared/tiler.py` | Phase 3 palettes requires multi-palette support |
| `services/tile-server/api/server.py` | Every phase adds API endpoints |
| `src/stores/useWeatherStore.ts` | Every phase adds state fields |
| `src/hooks/useManifest.ts` | Phase 2 unifies MRMS+nowcast+HRRR timeline |
| `src/app/(tabs)/radar.tsx` | Most UI features land here |
| `src/types/weather.ts` | Types evolve with each phase |

## Resource Requirements (Full Deployment)

| Category | CPU | RAM | Disk |
|---|---|---|---|
| Radar ingestors | 8 | 8 GB | 50 GB |
| NWP ingestors | 8 | 8 GB | 30 GB |
| Satellite + alerts | 4 | 4 GB | 12 GB |
| Hydro/surface/other | 4 | 4 GB | 10 GB |
| Nowcast + Open-Meteo | 6 | 10 GB | 500 GB |
| API + tiles + basemap | 2 | 2 GB | 40 GB |
| **Total** | **~32** | **~36 GB** | **~650 GB** |

Fits comfortably on the DL360 Gen9 Proxmox homelab.
