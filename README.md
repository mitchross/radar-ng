# StormScope

Hyper-local weather radar app with a self-hosted tile pipeline. CARROT Weather-inspired UI with weather-adaptive gradients, snarky personality, and glassmorphism cards.

Built with Expo SDK 55 (React Native) for the frontend and a Docker-based Python pipeline for the backend.

![App Screenshot](/docs/screenshots/hero.png)

---

## Architecture Overview

```
                              StormScope Architecture
  
  +------------------------------------------------------------------+
  |                        NOAA Public Data                          |
  |                                                                  |
  |   MRMS S3 Bucket              HRRR S3 Bucket                    |
  |   (2-min radar)               (hourly forecast)                  |
  |   s3://noaa-mrms-pds          s3://noaa-hrrr-bdp-pds            |
  +--------+--------------------------+------------------------------+
           |                          |
           v                          v
  +--------+---------+    +-----------+---------+
  |  ingest-mrms     |    |  ingest-hrrr        |
  |  (Python, 2min)  |    |  (Python, 60min)    |
  |                  |    |                     |
  |  GRIB2 decode    |    |  GRIB2 decode       |
  |  Color table     |    |  5 variables:       |
  |  Tile render     |    |  - reflectivity     |
  |  z4-z9           |    |  - temperature      |
  +--------+---------+    |  - wind (U+V)       |
           |              |  - CAPE              |
           |              |  - precip type       |
           |              |  Tile render z4-z9   |
           |              +-----------+----------+
           |                          |
           v                          v
  +-----------------------------------------------+
  |            Shared Volume: /data/tiles          |
  |                                                |
  |  /radar/{timestamp}/{z}/{x}/{y}.png            |
  |  /temperature/{timestamp}/{z}/{x}/{y}.png      |
  |  /wind/{timestamp}/{z}/{x}/{y}.png             |
  |  /cape/{timestamp}/{z}/{x}/{y}.png             |
  |  /precip-type/{timestamp}/{z}/{x}/{y}.png      |
  |  /radar-hrrr/{timestamp}/{z}/{x}/{y}.png       |
  +-----+---------------------------+--------------+
        |                           |
        v                           v
  +-----+----------+     +---------+---------+
  |  tile-cleanup   |     |  tile-server      |
  |  (30min cron)   |     |  Caddy + FastAPI  |
  |                 |     |  :8080            |
  |  radar: -4h     |     |                   |
  |  hrrr:  -8h     |     |  /tiles/* (static)|
  +-----------------+     |  /api/manifest    |
                          |  /api/forecast    |
                          |  /api/health      |
                          +--------+----------+
                                   |
                                   | HTTPS :8080
                                   v
  +-----------------------------------------------+
  |              Mobile App (Expo SDK 55)          |
  |                                                |
  |  +------------------+  +--------------------+  |
  |  | Weather Tab      |  | Radar Tab          |  |
  |  | (forecast hero)  |  | (MapLibre map)     |  |
  |  |                  |  |                    |  |
  |  | Gradient bg      |  | Tile overlays      |  |
  |  | Snarky quotes    |  | Timeline slider    |  |
  |  | Hourly chart     |  | Layer picker       |  |
  |  | 7-day forecast   |  | Alert polygons     |  |
  |  | Alert cards      |  | Play/pause         |  |
  |  +------------------+  +--------------------+  |
  |                                                |
  |  Data Sources:                                 |
  |   Free: IEM NEXRAD + Open-Meteo + NWS          |
  |   Self-Hosted: Your tile server pipeline       |
  +------------------------------------------------+
```

---

## How It Works

### Dual Data Source Model

StormScope runs in two modes:

**Free Tier** (no server required):
- **Radar tiles** from [IEM NEXRAD](https://mesonet.agron.iastate.edu/) — NWS-colored reflectivity, 50 minutes of history at 5-minute intervals
- **Forecasts** from [Open-Meteo](https://open-meteo.com/) — current conditions, 24h hourly, 7-day daily
- **Alerts** from [NWS API](https://api.weather.gov/) — active weather warnings with polygon geometry

**Self-Hosted Tier** (your server):
- **MRMS radar** — 2-minute updates, ~1km resolution, 4 hours of history
- **HRRR forecast layers** — temperature, wind, CAPE, precipitation type, reflectivity. Hourly updates with 24-hour forecast horizon
- **Forecast proxy** — Open-Meteo with 15-minute server-side cache
- Switch between modes in Settings

### Tile Pipeline

```
NOAA S3 (GRIB2)
     |
     v
[pygrib decode] -----> 2D numpy array (lat x lon grid)
     |
     v
[color table apply] -> RGBA uint8 array (NWS standard colors)
     |
     v
[mercantile math] ---> Slippy map tile coordinates (z/x/y)
     |
     v
[Pillow resize] -----> 256x256 PNG tiles per zoom level
     |
     v
/data/tiles/{layer}/{ISO-timestamp}/{z}/{x}/{y}.png
     |
     v
[Caddy static serve] -> HTTP :8080/tiles/...
     |
     v
[MapLibre RasterSource] -> rendered on map
```

Each ingest service downloads GRIB2 files from NOAA's public S3 buckets, decodes the meteorological data, applies a color lookup table (NWS standard colors), and renders XYZ slippy map tiles at zoom levels 4-9.

### Weather-Adaptive UI

The app changes its entire appearance based on current weather conditions:

| Condition | Background Gradient | Accent |
|-----------|-------------------|--------|
| Clear (day) | Blue sky (#1565C0 -> #42A5F5) | Gold |
| Clear (night) | Deep navy (#0D1B2A -> #2C3E50) | Light blue |
| Overcast | Slate gray (#455A64 -> #78909C) | Silver |
| Rain | Dark blue (#1A237E -> #1976D2) | Blue |
| Snow | Blue-gray (#546E7A -> #B0BEC5) | White |
| Thunderstorm | Deep purple (#1A0A2E -> #4A148C) | Lavender |

Temperature-adaptive font weight (CARROT Weather's signature): the hero temperature number gets **bolder as it gets warmer** and thinner as it gets colder.

---

## Data Sources

### MRMS (Multi-Radar Multi-Sensor)

| Property | Value |
|----------|-------|
| Source | `s3://noaa-mrms-pds/CONUS/MergedBaseReflectivity_00.50/` |
| Format | GRIB2 (gzip compressed) |
| Resolution | ~1 km (0.01 degree) |
| Update frequency | Every 2 minutes |
| Coverage | CONUS (Continental US) |
| Retention | 4 hours |
| Tile zooms | z4 - z9 |

### HRRR (High-Resolution Rapid Refresh)

| Property | Value |
|----------|-------|
| Source | `s3://noaa-hrrr-bdp-pds/hrrr.{date}/conus/` |
| Format | GRIB2 |
| Resolution | 3 km |
| Update frequency | Every hour |
| Forecast horizon | 24 hours |
| Retention | 8 hours |
| Tile zooms | z4 - z9 |

**Extracted variables:**

| Layer | GRIB2 Variable | Unit | Output |
|-------|---------------|------|--------|
| Radar (HRRR) | Composite reflectivity (entire atmosphere) | dBZ | `/tiles/radar-hrrr/` |
| Temperature | 2m temperature | Converted K -> F | `/tiles/temperature/` |
| Wind | 10m U-wind + V-wind | Computed speed, m/s -> mph | `/tiles/wind/` |
| CAPE | Convective Available Potential Energy | J/kg | `/tiles/cape/` |
| Precip Type | Categorical rain/snow/freezing rain/ice | Category ID | `/tiles/precip-type/` |

### Free Tier APIs

| API | URL | Rate | Auth |
|-----|-----|------|------|
| IEM NEXRAD | `mesonet.agron.iastate.edu/cache/tile.py/1.0.0/` | Unlimited | None |
| Open-Meteo | `api.open-meteo.com/v1/forecast` | 10k/day | None |
| NWS Alerts | `api.weather.gov/alerts/active` | Generous | User-Agent |

---

## Color Tables

All layers use NWS-standard color mappings defined in `services/shared/color_tables.json`:

### Reflectivity (dBZ)
```
  5 dBZ  ████  Light green    (light rain)
 20 dBZ  ████  Yellow         (moderate)
 35 dBZ  ████  Orange         (heavy)
 45 dBZ  ████  Red            (severe)
 55 dBZ  ████  Dark red       (hail risk)
 65 dBZ  ████  Magenta        (extreme)
```

### Temperature (F)
```
 -40F    ████  Purple         (extreme cold)
  15F    ████  Blue           (below freezing)
  32F    ████  Cyan           (freezing)
  55F    ████  Green          (cool)
  75F    ████  Yellow         (warm)
  90F    ████  Orange         (hot)
 100F+   ████  Red            (extreme heat)
```

### CAPE (J/kg)
```
 500     ████  Yellow         (marginal)
1500     ████  Orange         (moderate)
2500     ████  Orange-red     (high)
3500     ████  Red            (extreme)
4000+    ████  Magenta        (significant severe)
```

### Precipitation Type
```
 Rain             ████  Green
 Snow             ████  Blue
 Freezing Rain    ████  Pink
 Ice Pellets      ████  Purple
 Hail             ████  Orange
```

---

## Self-Hosting Guide

### Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| CPU | 2 cores | 4+ cores |
| RAM | 2 GB | 4+ GB |
| Disk | 10 GB free | 20+ GB SSD |
| Docker | 24.0+ | Latest |
| Network | 10 Mbps down | 50+ Mbps |

The pipeline downloads ~50-100 MB/hour from NOAA S3 (free, no auth). Tile generation is CPU-intensive during ingest bursts but idle between polls.

**Disk usage estimate:**
- MRMS radar tiles: ~200 MB per hour (4h retention = ~800 MB)
- HRRR layers (5 variables x 24 forecast hours): ~2 GB per run (8h retention = ~4 GB)
- **Total steady-state: ~5 GB**

### Quick Start

```bash
# Clone
git clone https://gitea.vanillax.me/vanillax/stormscope.git
cd stormscope

# Start the backend pipeline
cd deploy
docker compose up -d

# Verify it's running
curl http://localhost:8080/api/health
# {"status": "ok"}

# Check what data is available
curl http://localhost:8080/api/manifest.json | jq '.layers | keys'
# ["cape", "precip-type", "radar", "radar-hrrr", "temperature", "wind"]
```

MRMS tiles appear within ~2 minutes. HRRR tiles take ~5-10 minutes (large download).

### Docker Compose Architecture

```yaml
# deploy/docker-compose.yml
services:
  ingest-mrms:     # Polls NOAA MRMS S3 every 2 min
  ingest-hrrr:     # Polls NOAA HRRR S3 every 60 min
  tile-server:     # Caddy + FastAPI on :8080
  tile-cleanup:    # Removes expired tiles every 30 min

volumes:
  tiles:           # Shared /data/tiles across all services
```

```
+------------------+    +------------------+
|   ingest-mrms    |    |   ingest-hrrr    |
|   (every 2min)   |    |   (every 60min)  |
+--------+---------+    +--------+---------+
         |                       |
         v                       v
   +-----+-----------------------+------+
   |        tiles volume                |
   |        /data/tiles/                |
   +-----+-----------------------+------+
         |                       |
         v                       v
+--------+---------+    +--------+---------+
|   tile-server    |    |   tile-cleanup   |
|   :8080          |    |   (every 30min)  |
+------------------+    +------------------+
```

### API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/health` | GET | Health check |
| `/api/manifest.json` | GET | Available layers and timestamps |
| `/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png` | GET | Static tile serving (Caddy) |
| `/api/forecast/{lat}/{lon}` | GET | Open-Meteo proxy with 15min cache |

**Manifest response:**
```json
{
  "layers": {
    "radar": {
      "timestamps": ["2026-04-15T22:00:00+00:00", "2026-04-15T22:02:00+00:00", "..."]
    },
    "temperature": {
      "timestamps": ["2026-04-15T13:00:00+00:00", "2026-04-15T14:00:00+00:00", "..."]
    }
  },
  "tile_url_template": "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
  "updated_at": "2026-04-15T22:05:00Z"
}
```

### Connecting the App

1. Start the backend: `docker compose up -d`
2. Open StormScope on your phone
3. Go to **Settings** > **Data Source** > **Self-Hosted**
4. Enter your server URL (e.g., `http://192.168.1.100:8080`)
5. The app will fetch the manifest and display your tiles

For Android emulator, the server URL is `http://10.0.2.2:8080` (magic IP for host loopback).

---

## Building the App

### Prerequisites

- [Bun](https://bun.sh) 1.1+ (package manager + runner)
- Android SDK (API 35) or Xcode 16+
- Java 17 (for Android builds)

### Development

```bash
# Install dependencies
bun install

# Start Metro dev server
bun start

# Run on Android (requires emulator or device)
bun run android

# Run on iOS (macOS only)
bun run ios

# Run tests
bun test
```

### Production Build

```bash
# Android APK
bunx expo run:android --variant release

# Or use EAS Build
bunx eas build --platform android
```

---

## Project Structure

```
stormscope/
+-- src/
|   +-- app/                          # Expo Router (file-based routing)
|   |   +-- _layout.tsx               # Root: QueryClient + GestureHandler
|   |   +-- (tabs)/
|   |   |   +-- _layout.tsx           # Tab bar (Weather/Radar/Settings)
|   |   |   +-- index.tsx             # Weather tab (forecast hero)
|   |   |   +-- radar.tsx             # Radar tab (MapLibre map)
|   |   |   +-- settings.tsx          # Settings tab
|   |   +-- alert/[id].tsx            # Alert detail modal
|   +-- components/
|   |   +-- map/
|   |   |   +-- WeatherMap.tsx        # MapLibre wrapper
|   |   |   +-- RadarOverlay.tsx      # IEM/self-hosted tile source
|   |   |   +-- WeatherLayerOverlay.tsx  # Generic layer overlay
|   |   |   +-- AlertPolygon.tsx      # NWS alert geometry
|   |   +-- timeline/
|   |   |   +-- TimeSlider.tsx        # Frame timeline
|   |   |   +-- PlayButton.tsx        # Play/pause animation
|   |   +-- layers/
|   |   |   +-- LayerPicker.tsx       # FAB stack for layer selection
|   |   +-- alerts/
|   |       +-- AlertBanner.tsx       # Top alert banner
|   +-- hooks/
|   |   +-- useManifest.ts            # Dual-source manifest fetcher
|   |   +-- useForecast.ts            # Open-Meteo forecast hook
|   |   +-- useAlerts.ts              # NWS alerts hook
|   |   +-- useLocation.ts            # GPS location hook
|   +-- lib/
|   |   +-- api.ts                    # API functions
|   |   +-- constants.ts              # URLs, defaults, layer config
|   |   +-- storage.ts                # MMKV key-value persistence
|   |   +-- tileUrl.ts                # Tile URL builders
|   |   +-- weatherCodes.ts           # WMO code descriptions
|   |   +-- weatherTheme.ts           # Weather-adaptive theme system
|   +-- stores/
|   |   +-- useWeatherStore.ts        # Zustand global state
|   +-- types/
|       +-- weather.ts                # TypeScript interfaces
+-- services/
|   +-- shared/
|   |   +-- color_tables.json         # NWS color lookup tables
|   |   +-- tiler.py                  # Tile rendering engine
|   |   +-- test_tiler.py             # Tiler unit tests
|   +-- ingest-mrms/
|   |   +-- ingest.py                 # MRMS radar ingest loop
|   |   +-- Dockerfile
|   |   +-- requirements.txt
|   +-- ingest-hrrr/
|   |   +-- ingest.py                 # HRRR multi-variable ingest
|   |   +-- Dockerfile
|   |   +-- requirements.txt
|   +-- tile-server/
|   |   +-- Caddyfile                 # Static tiles + API proxy
|   |   +-- Dockerfile
|   |   +-- api/
|   |       +-- server.py             # FastAPI manifest + forecast proxy
|   |       +-- requirements.txt
|   +-- tile-cleanup/
|       +-- cleanup.sh                # Retention policy enforcement
+-- deploy/
|   +-- docker-compose.yml            # Full stack orchestration
+-- __tests__/                        # Jest unit tests
+-- android/                          # Native Android project
+-- app.json                          # Expo config
+-- package.json
+-- tsconfig.json
```

---

## Tech Stack

### Frontend
| Technology | Version | Purpose |
|-----------|---------|---------|
| Expo SDK | 55 | React Native framework |
| React Native | 0.83 | Mobile runtime |
| React | 19.2 | UI library |
| MapLibre Native | 10.4 | Map rendering |
| Zustand | 5.0 | State management |
| TanStack Query | 5.99 | Data fetching + caching |
| react-native-mmkv | 4.3 | Persistent key-value storage |
| expo-linear-gradient | 55.0 | Weather-adaptive backgrounds |
| expo-location | 55.1 | GPS positioning |
| TypeScript | 5.9 | Type safety |

### Backend
| Technology | Version | Purpose |
|-----------|---------|---------|
| Python | 3.12 | Ingest services |
| pygrib | 2.1+ | GRIB2 meteorological data decoding |
| numpy | 1.26+ | Array processing |
| Pillow | 10.0+ | PNG tile rendering |
| mercantile | (via tiler.py) | XYZ slippy map coordinate math |
| FastAPI | 0.115+ | Manifest API + forecast proxy |
| Caddy | 2.x | Static tile serving + reverse proxy |
| Docker Compose | 2.x | Service orchestration |

### Data Sources
| Source | Data | Update Rate | Auth |
|--------|------|-------------|------|
| NOAA MRMS (S3) | Base reflectivity | 2 min | None (public) |
| NOAA HRRR (S3) | Multi-variable forecast | 1 hr | None (public) |
| IEM NEXRAD | Free radar tiles | 5 min | None |
| Open-Meteo | Weather forecast | 15 min | None |
| NWS API | Active alerts | 1 min | User-Agent header |

---

## Resource Usage

### Backend (Docker)

| Service | CPU (idle) | CPU (ingest) | RAM | Disk I/O |
|---------|-----------|-------------|-----|----------|
| ingest-mrms | ~0% | 50-100% (burst) | ~200 MB | ~50 MB/cycle |
| ingest-hrrr | ~0% | 50-100% (burst) | ~500 MB | ~2 GB/cycle |
| tile-server | ~1% | ~5% (under load) | ~50 MB | Read-only |
| tile-cleanup | ~0% | ~1% (burst) | ~10 MB | Delete ops |

**Network bandwidth:** ~50-100 MB/hour download from NOAA S3

### Mobile App

| Metric | Value |
|--------|-------|
| APK size | ~147 MB (debug), ~50 MB (release) |
| RAM usage | ~150-200 MB |
| Battery | Minimal (map tiles cached, polling intervals >30s) |
| Network | ~5 MB/min during active radar playback |

---

## License

MIT
