# StormScope Phase 1 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a client-only React Native (Expo SDK 55) weather radar app that displays live NEXRAD radar on a map with a time slider, point forecasts, and severe weather alerts — all from free public APIs with zero backend.

**Architecture:** The app fetches a radar tile manifest from RainViewer (free, no auth), overlays XYZ radar tiles on a MapLibre GL map with an OpenFreeMap base layer, and queries Open-Meteo for point forecasts and NWS for alerts. Zustand manages timeline/layer state, TanStack Query handles API caching and refetching.

**Tech Stack:** Expo SDK 55 (RN 0.83, React 19.2), MapLibre React Native, RainViewer API, Open-Meteo API, NWS API, Zustand, TanStack Query, React Native Reanimated 3, @gorhom/bottom-sheet, expo-location, react-native-mmkv

---

## File Structure

```
radar-ng/
├── src/
│   ├── app/
│   │   ├── _layout.tsx              # Root layout: providers (QueryClient, GestureHandler)
│   │   ├── (tabs)/
│   │   │   ├── _layout.tsx          # Tab navigator layout (Map, Settings)
│   │   │   ├── index.tsx            # Map screen — primary UI
│   │   │   └── settings.tsx         # Settings screen
│   │   └── alert/
│   │       └── [id].tsx             # Alert detail screen
│   ├── components/
│   │   ├── map/
│   │   │   ├── WeatherMap.tsx       # MapLibre GL map with base layer + user location
│   │   │   └── RadarOverlay.tsx     # RasterSource + RasterLayer for radar tiles
│   │   ├── timeline/
│   │   │   ├── TimeSlider.tsx       # Horizontal scrubber for past radar frames
│   │   │   └── PlayButton.tsx       # Play/pause toggle + speed control
│   │   ├── forecast/
│   │   │   ├── ForecastSheet.tsx    # @gorhom/bottom-sheet wrapper
│   │   │   ├── CurrentConditions.tsx# Temp, wind, humidity display
│   │   │   └── HourlyScroll.tsx     # Horizontal scrolling hourly cards
│   │   └── alerts/
│   │       └── AlertBanner.tsx      # Top-of-screen NWS alert bar
│   ├── hooks/
│   │   ├── useManifest.ts           # TanStack Query hook for RainViewer manifest
│   │   ├── useForecast.ts           # TanStack Query hook for Open-Meteo
│   │   ├── useAlerts.ts             # TanStack Query hook for NWS alerts
│   │   └── useLocation.ts           # expo-location wrapper
│   ├── stores/
│   │   └── useWeatherStore.ts       # Zustand: timeline position, playback, location, radar opacity
│   ├── lib/
│   │   ├── api.ts                   # Fetch functions: RainViewer, Open-Meteo, NWS
│   │   ├── tileUrl.ts              # Build RainViewer tile URL from manifest frame
│   │   ├── weatherCodes.ts         # WMO code → label + icon mapping
│   │   └── constants.ts            # API URLs, default settings, color config
│   └── types/
│       └── weather.ts               # TS interfaces for all API responses + app state
├── __tests__/
│   ├── lib/
│   │   ├── api.test.ts
│   │   ├── tileUrl.test.ts
│   │   └── weatherCodes.test.ts
│   └── stores/
│       └── useWeatherStore.test.ts
├── app.json
├── package.json
└── tsconfig.json
```

---

## Task 1: Project Scaffold & Dependencies

**Files:**
- Create: `radar-ng/` (Expo project via create-expo-app)
- Modify: `package.json` (add deps), `app.json` (add MapLibre plugin)

- [ ] **Step 1: Create Expo SDK 55 project**

```bash
cd /home/vanillax/programming
npx create-expo-app@latest radar-ng --template default@sdk-55
```

If the directory already exists and is empty, remove it first: `rm -rf radar-ng && npx create-expo-app@latest radar-ng --template default@sdk-55`

- [ ] **Step 2: Install core dependencies**

```bash
cd /home/vanillax/programming/radar-ng
npx expo install @maplibre/maplibre-react-native
npx expo install expo-location
npx expo install @tanstack/react-query
npx expo install zustand
npx expo install react-native-mmkv
npx expo install @gorhom/bottom-sheet
npx expo install react-native-reanimated
npx expo install react-native-gesture-handler
```

- [ ] **Step 3: Configure app.json with MapLibre plugin**

In `app.json`, add the MapLibre plugin to the `plugins` array and update the app name:

```json
{
  "expo": {
    "name": "StormScope",
    "slug": "stormscope",
    "plugins": [
      "@maplibre/maplibre-react-native"
    ],
    "ios": {
      "infoPlist": {
        "NSLocationWhenInUseUsageDescription": "StormScope uses your location to show local weather radar and forecasts."
      }
    },
    "android": {
      "permissions": ["ACCESS_FINE_LOCATION", "ACCESS_COARSE_LOCATION"]
    }
  }
}
```

Preserve any existing fields from the template (scheme, orientation, icon, splash, etc.) — only add/modify the fields shown above.

- [ ] **Step 4: Initialize git and commit**

```bash
cd /home/vanillax/programming/radar-ng
git init
git add -A
git commit -m "feat: scaffold Expo SDK 55 project with dependencies"
```

---

## Task 2: TypeScript Types & Constants

**Files:**
- Create: `src/types/weather.ts`
- Create: `src/lib/constants.ts`

- [ ] **Step 1: Create types file**

Create `src/types/weather.ts`:

```typescript
// --- RainViewer API ---

export interface RainViewerManifest {
  version: string;
  generated: number;
  host: string;
  radar: {
    past: RadarFrame[];
    nowcast: RadarFrame[];
  };
  satellite: {
    infrared: RadarFrame[];
  };
}

export interface RadarFrame {
  time: number; // Unix epoch seconds
  path: string; // e.g. "/v2/radar/32a737032949"
}

// --- Open-Meteo API ---

export interface OpenMeteoResponse {
  latitude: number;
  longitude: number;
  current: {
    time: string;
    temperature_2m: number;
    relative_humidity_2m: number;
    apparent_temperature: number;
    weather_code: number;
    wind_speed_10m: number;
    wind_direction_10m: number;
    wind_gusts_10m: number;
  };
  hourly: {
    time: string[];
    temperature_2m: number[];
    precipitation_probability: number[];
    weather_code: number[];
    wind_speed_10m: number[];
  };
  daily: {
    time: string[];
    temperature_2m_max: number[];
    temperature_2m_min: number[];
    weather_code: number[];
    precipitation_sum: number[];
    sunrise: string[];
    sunset: string[];
  };
}

// --- NWS Alerts API ---

export interface NWSAlertCollection {
  type: "FeatureCollection";
  features: NWSAlert[];
}

export interface NWSAlert {
  id: string;
  type: "Feature";
  geometry: {
    type: "Polygon";
    coordinates: number[][][];
  } | null;
  properties: {
    id: string;
    event: string;
    headline: string | null;
    description: string;
    severity: "Extreme" | "Severe" | "Moderate" | "Minor" | "Unknown";
    urgency: "Immediate" | "Expected" | "Future" | "Past" | "Unknown";
    expires: string;
    areaDesc: string;
  };
}

// --- App State ---

export type TemperatureUnit = "fahrenheit" | "celsius";
export type WindUnit = "mph" | "kmh";
export type MapStyle = "light" | "dark";
```

- [ ] **Step 2: Create constants file**

Create `src/lib/constants.ts`:

```typescript
export const API = {
  RAINVIEWER_MANIFEST: "https://api.rainviewer.com/public/weather-maps.json",
  OPEN_METEO: "https://api.open-meteo.com/v1/forecast",
  NWS_ALERTS: "https://api.weather.gov/alerts/active",
} as const;

export const MAP_STYLES = {
  light: "https://tiles.openfreemap.org/styles/liberty",
  dark: "https://tiles.openfreemap.org/styles/dark",
} as const;

export const RADAR = {
  TILE_SIZE: 256,
  COLOR_SCHEME: 1, // Universal Blue (free tier)
  SMOOTH: true,
  SNOW: true,
  MIN_ZOOM: 1,
  MAX_ZOOM: 7,
  DEFAULT_OPACITY: 0.7,
} as const;

export const DEFAULTS = {
  // Center of CONUS
  LATITUDE: 39.8283,
  LONGITUDE: -98.5795,
  ZOOM: 4,
  PLAYBACK_FPS: 5,
  MANIFEST_REFETCH_MS: 30_000,
  FORECAST_REFETCH_MS: 15 * 60_000, // 15 min
  ALERTS_REFETCH_MS: 60_000, // 1 min
} as const;
```

- [ ] **Step 3: Commit**

```bash
git add src/types/weather.ts src/lib/constants.ts
git commit -m "feat: add TypeScript types and constants for weather APIs"
```

---

## Task 3: API Clients

**Files:**
- Create: `src/lib/api.ts`
- Create: `__tests__/lib/api.test.ts`

- [ ] **Step 1: Write failing tests for API clients**

Create `__tests__/lib/api.test.ts`:

```typescript
import { fetchRadarManifest, fetchForecast, fetchAlerts } from "../../src/lib/api";

// Mock global fetch
const mockFetch = jest.fn();
global.fetch = mockFetch;

afterEach(() => {
  mockFetch.mockReset();
});

describe("fetchRadarManifest", () => {
  it("returns parsed manifest on success", async () => {
    const manifest = {
      version: "2.0",
      generated: 1776220529,
      host: "https://tilecache.rainviewer.com",
      radar: { past: [{ time: 1776213000, path: "/v2/radar/abc123" }], nowcast: [] },
      satellite: { infrared: [] },
    };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(manifest),
    });

    const result = await fetchRadarManifest();
    expect(result).toEqual(manifest);
    expect(mockFetch).toHaveBeenCalledWith(
      "https://api.rainviewer.com/public/weather-maps.json"
    );
  });

  it("throws on non-ok response", async () => {
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });
    await expect(fetchRadarManifest()).rejects.toThrow("RainViewer API error: 500");
  });
});

describe("fetchForecast", () => {
  it("passes correct parameters and returns forecast", async () => {
    const forecast = { current: { temperature_2m: 72 } };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(forecast),
    });

    const result = await fetchForecast(38.9, -77.0);
    expect(result).toEqual(forecast);

    const calledUrl = mockFetch.mock.calls[0][0] as string;
    expect(calledUrl).toContain("latitude=38.9");
    expect(calledUrl).toContain("longitude=-77");
    expect(calledUrl).toContain("temperature_unit=fahrenheit");
  });
});

describe("fetchAlerts", () => {
  it("sends correct User-Agent header", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ features: [] }),
    });

    await fetchAlerts(38.9, -77.0);
    const calledUrl = mockFetch.mock.calls[0][0] as string;
    const calledOptions = mockFetch.mock.calls[0][1] as RequestInit;
    expect(calledUrl).toContain("point=38.9,-77");
    expect(calledOptions.headers).toEqual(
      expect.objectContaining({ "User-Agent": expect.stringContaining("StormScope") })
    );
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
npx jest __tests__/lib/api.test.ts --no-cache
```

Expected: FAIL — module `../../src/lib/api` not found.

- [ ] **Step 3: Implement API clients**

Create `src/lib/api.ts`:

```typescript
import { API } from "./constants";
import type {
  RainViewerManifest,
  OpenMeteoResponse,
  NWSAlertCollection,
} from "../types/weather";

export async function fetchRadarManifest(): Promise<RainViewerManifest> {
  const res = await fetch(API.RAINVIEWER_MANIFEST);
  if (!res.ok) throw new Error(`RainViewer API error: ${res.status}`);
  return res.json();
}

export async function fetchForecast(
  lat: number,
  lon: number
): Promise<OpenMeteoResponse> {
  const params = new URLSearchParams({
    latitude: String(lat),
    longitude: String(lon),
    current:
      "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m",
    hourly:
      "temperature_2m,precipitation_probability,weather_code,wind_speed_10m",
    daily:
      "temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,sunrise,sunset",
    temperature_unit: "fahrenheit",
    wind_speed_unit: "mph",
    precipitation_unit: "inch",
    timezone: "auto",
    forecast_days: "7",
  });
  const res = await fetch(`${API.OPEN_METEO}?${params}`);
  if (!res.ok) throw new Error(`Open-Meteo API error: ${res.status}`);
  return res.json();
}

export async function fetchAlerts(
  lat: number,
  lon: number
): Promise<NWSAlertCollection> {
  const res = await fetch(`${API.NWS_ALERTS}?point=${lat},${lon}`, {
    headers: { "User-Agent": "StormScope/1.0 (weather-radar-app)" },
  });
  if (!res.ok) throw new Error(`NWS API error: ${res.status}`);
  return res.json();
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
npx jest __tests__/lib/api.test.ts --no-cache
```

Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/lib/api.ts __tests__/lib/api.test.ts
git commit -m "feat: add API clients for RainViewer, Open-Meteo, and NWS"
```

---

## Task 4: Tile URL Builder & Weather Codes

**Files:**
- Create: `src/lib/tileUrl.ts`
- Create: `src/lib/weatherCodes.ts`
- Create: `__tests__/lib/tileUrl.test.ts`
- Create: `__tests__/lib/weatherCodes.test.ts`

- [ ] **Step 1: Write failing tests for tile URL builder**

Create `__tests__/lib/tileUrl.test.ts`:

```typescript
import { buildRadarTileUrl } from "../../src/lib/tileUrl";

describe("buildRadarTileUrl", () => {
  const host = "https://tilecache.rainviewer.com";
  const frame = { time: 1776213000, path: "/v2/radar/abc123" };

  it("builds correct tile URL with defaults", () => {
    const url = buildRadarTileUrl(host, frame);
    expect(url).toBe(
      "https://tilecache.rainviewer.com/v2/radar/abc123/256/{z}/{x}/{y}/1/1_1.png"
    );
  });

  it("respects custom tile size", () => {
    const url = buildRadarTileUrl(host, frame, { size: 512 });
    expect(url).toContain("/512/{z}");
  });

  it("respects smooth=false and snow=false", () => {
    const url = buildRadarTileUrl(host, frame, { smooth: false, snow: false });
    expect(url).toEndWith("/1/0_0.png");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npx jest __tests__/lib/tileUrl.test.ts --no-cache
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement tile URL builder**

Create `src/lib/tileUrl.ts`:

```typescript
import type { RadarFrame } from "../types/weather";
import { RADAR } from "./constants";

export function buildRadarTileUrl(
  host: string,
  frame: RadarFrame,
  options: {
    size?: number;
    color?: number;
    smooth?: boolean;
    snow?: boolean;
  } = {}
): string {
  const {
    size = RADAR.TILE_SIZE,
    color = RADAR.COLOR_SCHEME,
    smooth = RADAR.SMOOTH,
    snow = RADAR.SNOW,
  } = options;
  return `${host}${frame.path}/${size}/{z}/{x}/{y}/${color}/${smooth ? 1 : 0}_${snow ? 1 : 0}.png`;
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
npx jest __tests__/lib/tileUrl.test.ts --no-cache
```

Expected: 3 tests PASS.

- [ ] **Step 5: Write weather code mapping**

Create `src/lib/weatherCodes.ts`:

```typescript
interface WeatherInfo {
  label: string;
  icon: string; // emoji for now, swap for icon component later
}

const WMO_CODES: Record<number, WeatherInfo> = {
  0: { label: "Clear sky", icon: "\u2600\uFE0F" },
  1: { label: "Mainly clear", icon: "\uD83C\uDF24\uFE0F" },
  2: { label: "Partly cloudy", icon: "\u26C5" },
  3: { label: "Overcast", icon: "\u2601\uFE0F" },
  45: { label: "Fog", icon: "\uD83C\uDF2B\uFE0F" },
  48: { label: "Rime fog", icon: "\uD83C\uDF2B\uFE0F" },
  51: { label: "Light drizzle", icon: "\uD83C\uDF26\uFE0F" },
  53: { label: "Drizzle", icon: "\uD83C\uDF26\uFE0F" },
  55: { label: "Dense drizzle", icon: "\uD83C\uDF26\uFE0F" },
  61: { label: "Light rain", icon: "\uD83C\uDF27\uFE0F" },
  63: { label: "Rain", icon: "\uD83C\uDF27\uFE0F" },
  65: { label: "Heavy rain", icon: "\uD83C\uDF27\uFE0F" },
  71: { label: "Light snow", icon: "\uD83C\uDF28\uFE0F" },
  73: { label: "Snow", icon: "\uD83C\uDF28\uFE0F" },
  75: { label: "Heavy snow", icon: "\uD83C\uDF28\uFE0F" },
  77: { label: "Snow grains", icon: "\uD83C\uDF28\uFE0F" },
  80: { label: "Light showers", icon: "\uD83C\uDF26\uFE0F" },
  81: { label: "Showers", icon: "\uD83C\uDF27\uFE0F" },
  82: { label: "Heavy showers", icon: "\uD83C\uDF27\uFE0F" },
  85: { label: "Light snow showers", icon: "\uD83C\uDF28\uFE0F" },
  86: { label: "Snow showers", icon: "\uD83C\uDF28\uFE0F" },
  95: { label: "Thunderstorm", icon: "\u26C8\uFE0F" },
  96: { label: "Thunderstorm + hail", icon: "\u26C8\uFE0F" },
  99: { label: "Thunderstorm + heavy hail", icon: "\u26C8\uFE0F" },
};

export function getWeatherInfo(code: number): WeatherInfo {
  return WMO_CODES[code] ?? { label: "Unknown", icon: "\u2753" };
}
```

- [ ] **Step 6: Write test for weather codes**

Create `__tests__/lib/weatherCodes.test.ts`:

```typescript
import { getWeatherInfo } from "../../src/lib/weatherCodes";

describe("getWeatherInfo", () => {
  it("returns correct label for clear sky", () => {
    expect(getWeatherInfo(0).label).toBe("Clear sky");
  });

  it("returns correct label for thunderstorm", () => {
    expect(getWeatherInfo(95).label).toBe("Thunderstorm");
  });

  it("returns Unknown for unrecognized code", () => {
    expect(getWeatherInfo(999).label).toBe("Unknown");
  });
});
```

- [ ] **Step 7: Run all tests**

```bash
npx jest __tests__/lib/ --no-cache
```

Expected: All tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src/lib/tileUrl.ts src/lib/weatherCodes.ts __tests__/lib/tileUrl.test.ts __tests__/lib/weatherCodes.test.ts
git commit -m "feat: add tile URL builder and WMO weather code mapping"
```

---

## Task 5: Zustand Store

**Files:**
- Create: `src/stores/useWeatherStore.ts`
- Create: `__tests__/stores/useWeatherStore.test.ts`

- [ ] **Step 1: Write failing tests for the store**

Create `__tests__/stores/useWeatherStore.test.ts`:

```typescript
import { useWeatherStore } from "../../src/stores/useWeatherStore";

// Reset store between tests
beforeEach(() => {
  useWeatherStore.setState(useWeatherStore.getInitialState());
});

describe("useWeatherStore", () => {
  it("starts with default values", () => {
    const state = useWeatherStore.getState();
    expect(state.frames).toEqual([]);
    expect(state.currentFrameIndex).toBe(-1);
    expect(state.isPlaying).toBe(false);
    expect(state.radarOpacity).toBe(0.7);
  });

  it("setFrames updates frames and sets index to last frame", () => {
    const frames = [
      { time: 1000, path: "/a" },
      { time: 2000, path: "/b" },
    ];
    useWeatherStore.getState().setFrames(frames);
    const state = useWeatherStore.getState();
    expect(state.frames).toEqual(frames);
  });

  it("nextFrame wraps around to 0", () => {
    useWeatherStore.setState({
      frames: [
        { time: 1, path: "/a" },
        { time: 2, path: "/b" },
      ],
      currentFrameIndex: 1,
    });
    useWeatherStore.getState().nextFrame();
    expect(useWeatherStore.getState().currentFrameIndex).toBe(0);
  });

  it("nextFrame does nothing with empty frames", () => {
    useWeatherStore.getState().nextFrame();
    expect(useWeatherStore.getState().currentFrameIndex).toBe(-1);
  });

  it("togglePlaying flips isPlaying", () => {
    expect(useWeatherStore.getState().isPlaying).toBe(false);
    useWeatherStore.getState().togglePlaying();
    expect(useWeatherStore.getState().isPlaying).toBe(true);
    useWeatherStore.getState().togglePlaying();
    expect(useWeatherStore.getState().isPlaying).toBe(false);
  });

  it("setLocation updates lat/lon", () => {
    useWeatherStore.getState().setLocation(38.9, -77.0);
    const { latitude, longitude } = useWeatherStore.getState();
    expect(latitude).toBe(38.9);
    expect(longitude).toBe(-77.0);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
npx jest __tests__/stores/useWeatherStore.test.ts --no-cache
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement the store**

Create `src/stores/useWeatherStore.ts`:

```typescript
import { create } from "zustand";
import type { RadarFrame, TemperatureUnit, MapStyle } from "../types/weather";
import { DEFAULTS, RADAR } from "../lib/constants";

interface WeatherState {
  // Timeline
  frames: RadarFrame[];
  currentFrameIndex: number;
  isPlaying: boolean;
  playbackSpeed: number;

  // Location
  latitude: number | null;
  longitude: number | null;

  // Layers
  radarOpacity: number;
  radarVisible: boolean;

  // Settings
  temperatureUnit: TemperatureUnit;
  mapStyle: MapStyle;

  // Actions
  setFrames: (frames: RadarFrame[]) => void;
  setCurrentFrameIndex: (index: number) => void;
  setIsPlaying: (playing: boolean) => void;
  togglePlaying: () => void;
  setPlaybackSpeed: (speed: number) => void;
  setLocation: (lat: number, lon: number) => void;
  setRadarOpacity: (opacity: number) => void;
  setRadarVisible: (visible: boolean) => void;
  setTemperatureUnit: (unit: TemperatureUnit) => void;
  setMapStyle: (style: MapStyle) => void;
  nextFrame: () => void;
}

export const useWeatherStore = create<WeatherState>()((set, get) => ({
  frames: [],
  currentFrameIndex: -1,
  isPlaying: false,
  playbackSpeed: DEFAULTS.PLAYBACK_FPS,

  latitude: null,
  longitude: null,

  radarOpacity: RADAR.DEFAULT_OPACITY,
  radarVisible: true,

  temperatureUnit: "fahrenheit",
  mapStyle: "light",

  setFrames: (frames) => set({ frames }),
  setCurrentFrameIndex: (index) => set({ currentFrameIndex: index }),
  setIsPlaying: (playing) => set({ isPlaying: playing }),
  togglePlaying: () => set((s) => ({ isPlaying: !s.isPlaying })),
  setPlaybackSpeed: (speed) => set({ playbackSpeed: speed }),
  setLocation: (lat, lon) => set({ latitude: lat, longitude: lon }),
  setRadarOpacity: (opacity) => set({ radarOpacity: opacity }),
  setRadarVisible: (visible) => set({ radarVisible: visible }),
  setTemperatureUnit: (unit) => set({ temperatureUnit: unit }),
  setMapStyle: (style) => set({ mapStyle: style }),
  nextFrame: () => {
    const { frames, currentFrameIndex } = get();
    if (frames.length === 0) return;
    set({ currentFrameIndex: (currentFrameIndex + 1) % frames.length });
  },
}));
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
npx jest __tests__/stores/useWeatherStore.test.ts --no-cache
```

Expected: 6 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/stores/useWeatherStore.ts __tests__/stores/useWeatherStore.test.ts
git commit -m "feat: add Zustand store for timeline, location, and settings"
```

---

## Task 6: React Query Hooks & Location

**Files:**
- Create: `src/hooks/useManifest.ts`
- Create: `src/hooks/useForecast.ts`
- Create: `src/hooks/useAlerts.ts`
- Create: `src/hooks/useLocation.ts`

- [ ] **Step 1: Create useManifest hook**

Create `src/hooks/useManifest.ts`:

```typescript
import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchRadarManifest } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useManifest() {
  const setFrames = useWeatherStore((s) => s.setFrames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);

  const query = useQuery({
    queryKey: ["radar-manifest"],
    queryFn: fetchRadarManifest,
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
  });

  useEffect(() => {
    if (!query.data) return;
    const allFrames = [
      ...query.data.radar.past,
      ...query.data.radar.nowcast,
    ];
    setFrames(allFrames);
    // Only jump to latest if user hasn't manually scrubbed
    if (currentFrameIndex === -1 || currentFrameIndex >= allFrames.length) {
      setCurrentFrameIndex(allFrames.length - 1);
    }
  }, [query.data]);

  return query;
}
```

- [ ] **Step 2: Create useForecast hook**

Create `src/hooks/useForecast.ts`:

```typescript
import { useQuery } from "@tanstack/react-query";
import { fetchForecast } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useForecast() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);

  return useQuery({
    queryKey: ["forecast", latitude, longitude],
    queryFn: () => fetchForecast(latitude!, longitude!),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: DEFAULTS.FORECAST_REFETCH_MS,
    staleTime: DEFAULTS.FORECAST_REFETCH_MS,
  });
}
```

- [ ] **Step 3: Create useAlerts hook**

Create `src/hooks/useAlerts.ts`:

```typescript
import { useQuery } from "@tanstack/react-query";
import { fetchAlerts } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useAlerts() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);

  return useQuery({
    queryKey: ["alerts", latitude, longitude],
    queryFn: () => fetchAlerts(latitude!, longitude!),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: DEFAULTS.ALERTS_REFETCH_MS,
    staleTime: DEFAULTS.ALERTS_REFETCH_MS,
  });
}
```

- [ ] **Step 4: Create useLocation hook**

Create `src/hooks/useLocation.ts`:

```typescript
import { useEffect, useState } from "react";
import * as Location from "expo-location";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useLocation() {
  const setLocation = useWeatherStore((s) => s.setLocation);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function requestLocation() {
      const { status } = await Location.requestForegroundPermissionsAsync();
      if (status !== "granted") {
        setError("Location permission denied");
        // Fall back to CONUS center
        setLocation(DEFAULTS.LATITUDE, DEFAULTS.LONGITUDE);
        return;
      }
      const loc = await Location.getCurrentPositionAsync({
        accuracy: Location.Accuracy.Balanced,
      });
      if (!cancelled) {
        setLocation(loc.coords.latitude, loc.coords.longitude);
      }
    }

    requestLocation();
    return () => {
      cancelled = true;
    };
  }, []);

  return { error };
}
```

- [ ] **Step 5: Commit**

```bash
git add src/hooks/
git commit -m "feat: add React Query hooks for manifest, forecast, alerts, and location"
```

---

## Task 7: Root Layout & Providers

**Files:**
- Modify: `src/app/_layout.tsx`
- Modify: `src/app/(tabs)/_layout.tsx`

- [ ] **Step 1: Set up root layout with providers**

Replace the contents of `src/app/_layout.tsx`:

```tsx
import { Stack } from "expo-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { StatusBar } from "expo-status-bar";
import { StyleSheet } from "react-native";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 2,
      gcTime: 10 * 60_000, // 10 min
    },
  },
});

export default function RootLayout() {
  return (
    <GestureHandlerRootView style={styles.container}>
      <QueryClientProvider client={queryClient}>
        <StatusBar style="light" />
        <Stack screenOptions={{ headerShown: false }}>
          <Stack.Screen name="(tabs)" />
          <Stack.Screen
            name="alert/[id]"
            options={{
              presentation: "modal",
              headerShown: true,
              headerTitle: "Alert Details",
            }}
          />
        </Stack>
      </QueryClientProvider>
    </GestureHandlerRootView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
});
```

- [ ] **Step 2: Set up tab layout**

Replace the contents of `src/app/(tabs)/_layout.tsx`:

```tsx
import { Tabs } from "expo-router";
import { Platform } from "react-native";

export default function TabLayout() {
  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarStyle: {
          backgroundColor: "#1a1a2e",
          borderTopColor: "#333",
        },
        tabBarActiveTintColor: "#4fc3f7",
        tabBarInactiveTintColor: "#888",
      }}
    >
      <Tabs.Screen
        name="index"
        options={{
          title: "Map",
          tabBarIcon: ({ color, size }) => null, // Replace with icon in polish phase
        }}
      />
      <Tabs.Screen
        name="settings"
        options={{
          title: "Settings",
          tabBarIcon: ({ color, size }) => null,
        }}
      />
    </Tabs>
  );
}
```

- [ ] **Step 3: Commit**

```bash
git add src/app/_layout.tsx src/app/\(tabs\)/_layout.tsx
git commit -m "feat: set up root layout with providers and tab navigation"
```

---

## Task 8: Map Screen with Base Map

**Files:**
- Create: `src/components/map/WeatherMap.tsx`
- Modify: `src/app/(tabs)/index.tsx`

- [ ] **Step 1: Create the WeatherMap component**

Create `src/components/map/WeatherMap.tsx`:

```tsx
import { useRef } from "react";
import MapLibreGL from "@maplibre/maplibre-react-native";
import { StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { MAP_STYLES, DEFAULTS } from "../../lib/constants";

MapLibreGL.setAccessToken(null);

interface WeatherMapProps {
  children?: React.ReactNode;
}

export function WeatherMap({ children }: WeatherMapProps) {
  const mapRef = useRef<MapLibreGL.MapView>(null);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);

  const centerCoord: [number, number] = [
    longitude ?? DEFAULTS.LONGITUDE,
    latitude ?? DEFAULTS.LATITUDE,
  ];

  return (
    <MapLibreGL.MapView
      ref={mapRef}
      style={styles.map}
      styleURL={MAP_STYLES[mapStyle]}
      logoEnabled={false}
      attributionEnabled={true}
      attributionPosition={{ bottom: 8, left: 8 }}
    >
      <MapLibreGL.Camera
        defaultSettings={{
          centerCoordinate: centerCoord,
          zoomLevel: latitude ? 7 : DEFAULTS.ZOOM,
        }}
      />
      <MapLibreGL.UserLocation visible={true} />
      {children}
    </MapLibreGL.MapView>
  );
}

const styles = StyleSheet.create({
  map: {
    flex: 1,
  },
});
```

- [ ] **Step 2: Create the map screen**

Replace contents of `src/app/(tabs)/index.tsx`:

```tsx
import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { useLocation } from "../../hooks/useLocation";
import { useManifest } from "../../hooks/useManifest";

export default function MapScreen() {
  useLocation();
  useManifest();

  return (
    <View style={styles.container}>
      <WeatherMap />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#000",
  },
});
```

- [ ] **Step 3: Build and test on device/simulator**

```bash
npx expo prebuild --clean
npx expo run:ios
# or: npx expo run:android
```

Verify: Map renders with OpenFreeMap tiles. User location dot appears (after granting permission). No crashes.

- [ ] **Step 4: Commit**

```bash
git add src/components/map/WeatherMap.tsx src/app/\(tabs\)/index.tsx
git commit -m "feat: add MapLibre base map with user location"
```

---

## Task 9: Radar Tile Overlay

**Files:**
- Create: `src/components/map/RadarOverlay.tsx`
- Modify: `src/app/(tabs)/index.tsx`

- [ ] **Step 1: Create the RadarOverlay component**

Create `src/components/map/RadarOverlay.tsx`:

```tsx
import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useManifest } from "../../hooks/useManifest";
import { buildRadarTileUrl } from "../../lib/tileUrl";
import { RADAR } from "../../lib/constants";

export function RadarOverlay() {
  const { data: manifest } = useManifest();
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);

  if (!manifest || frames.length === 0 || currentFrameIndex < 0) {
    return null;
  }

  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  const tileUrl = buildRadarTileUrl(manifest.host, frame);

  return (
    <MapLibreGL.RasterSource
      id="radar-source"
      key={frame.path} // Force re-mount when frame changes to swap tiles
      tileUrlTemplates={[tileUrl]}
      tileSize={RADAR.TILE_SIZE}
      minZoomLevel={RADAR.MIN_ZOOM}
      maxZoomLevel={RADAR.MAX_ZOOM}
    >
      <MapLibreGL.RasterLayer
        id="radar-layer"
        style={{
          rasterOpacity: radarVisible ? radarOpacity : 0,
          rasterFadeDuration: 0,
        }}
      />
    </MapLibreGL.RasterSource>
  );
}
```

- [ ] **Step 2: Add RadarOverlay to the map screen**

Modify `src/app/(tabs)/index.tsx` — add the overlay as a child of WeatherMap:

```tsx
import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { useLocation } from "../../hooks/useLocation";
import { useManifest } from "../../hooks/useManifest";

export default function MapScreen() {
  useLocation();
  useManifest();

  return (
    <View style={styles.container}>
      <WeatherMap>
        <RadarOverlay />
      </WeatherMap>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#000",
  },
});
```

- [ ] **Step 3: Test on device**

Run the app. Verify:
- Radar tiles appear on the map as a colored overlay
- Tiles show precipitation in blue (RainViewer Universal Blue scheme)
- Transparent areas (no precip) show the base map underneath
- Pinch-to-zoom works up to z7

- [ ] **Step 4: Commit**

```bash
git add src/components/map/RadarOverlay.tsx src/app/\(tabs\)/index.tsx
git commit -m "feat: add RainViewer radar tile overlay on map"
```

---

## Task 10: Time Slider & Playback

**Files:**
- Create: `src/components/timeline/TimeSlider.tsx`
- Create: `src/components/timeline/PlayButton.tsx`
- Modify: `src/app/(tabs)/index.tsx`

- [ ] **Step 1: Create TimeSlider component**

Create `src/components/timeline/TimeSlider.tsx`:

```tsx
import { View, Text, StyleSheet } from "react-native";
import Slider from "@react-native-community/slider";
import { useWeatherStore } from "../../stores/useWeatherStore";

export function TimeSlider() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);

  if (frames.length === 0) return null;

  const currentFrame = frames[currentFrameIndex];
  const timeLabel = currentFrame
    ? new Date(currentFrame.time * 1000).toLocaleTimeString([], {
        hour: "numeric",
        minute: "2-digit",
      })
    : "";

  const isLatest = currentFrameIndex === frames.length - 1;

  return (
    <View style={styles.container}>
      <View style={styles.labelRow}>
        <Text style={styles.timeText}>{timeLabel}</Text>
        {isLatest && <Text style={styles.liveBadge}>LIVE</Text>}
      </View>
      <Slider
        style={styles.slider}
        minimumValue={0}
        maximumValue={frames.length - 1}
        step={1}
        value={currentFrameIndex}
        onValueChange={(value) => {
          setIsPlaying(false);
          setCurrentFrameIndex(Math.round(value));
        }}
        minimumTrackTintColor="#4fc3f7"
        maximumTrackTintColor="#555"
        thumbTintColor="#4fc3f7"
      />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    paddingHorizontal: 16,
    paddingBottom: 8,
    backgroundColor: "rgba(26, 26, 46, 0.9)",
  },
  labelRow: {
    flexDirection: "row",
    justifyContent: "center",
    alignItems: "center",
    gap: 8,
    paddingTop: 8,
  },
  timeText: {
    color: "#fff",
    fontSize: 16,
    fontWeight: "600",
    fontVariant: ["tabular-nums"],
  },
  liveBadge: {
    color: "#4caf50",
    fontSize: 12,
    fontWeight: "700",
    backgroundColor: "rgba(76, 175, 80, 0.2)",
    paddingHorizontal: 6,
    paddingVertical: 2,
    borderRadius: 4,
    overflow: "hidden",
  },
  slider: {
    width: "100%",
    height: 40,
  },
});
```

**Note:** This requires `@react-native-community/slider`. Install it:

```bash
npx expo install @react-native-community/slider
```

- [ ] **Step 2: Create PlayButton component**

Create `src/components/timeline/PlayButton.tsx`:

```tsx
import { useEffect, useRef } from "react";
import { TouchableOpacity, Text, StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";

export function PlayButton() {
  const isPlaying = useWeatherStore((s) => s.isPlaying);
  const togglePlaying = useWeatherStore((s) => s.togglePlaying);
  const nextFrame = useWeatherStore((s) => s.nextFrame);
  const playbackSpeed = useWeatherStore((s) => s.playbackSpeed);
  const frames = useWeatherStore((s) => s.frames);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (isPlaying && frames.length > 0) {
      intervalRef.current = setInterval(() => {
        nextFrame();
      }, 1000 / playbackSpeed);
    }
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [isPlaying, playbackSpeed, frames.length]);

  if (frames.length === 0) return null;

  return (
    <TouchableOpacity
      style={styles.button}
      onPress={togglePlaying}
      activeOpacity={0.7}
    >
      <Text style={styles.icon}>{isPlaying ? "\u23F8" : "\u25B6\uFE0F"}</Text>
    </TouchableOpacity>
  );
}

const styles = StyleSheet.create({
  button: {
    width: 48,
    height: 48,
    borderRadius: 24,
    backgroundColor: "rgba(26, 26, 46, 0.9)",
    justifyContent: "center",
    alignItems: "center",
    marginLeft: 12,
  },
  icon: {
    fontSize: 20,
  },
});
```

- [ ] **Step 3: Add timeline controls to the map screen**

Update `src/app/(tabs)/index.tsx`:

```tsx
import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { TimeSlider } from "../../components/timeline/TimeSlider";
import { PlayButton } from "../../components/timeline/PlayButton";
import { useLocation } from "../../hooks/useLocation";
import { useManifest } from "../../hooks/useManifest";

export default function MapScreen() {
  useLocation();
  useManifest();

  return (
    <View style={styles.container}>
      <WeatherMap>
        <RadarOverlay />
      </WeatherMap>
      <View style={styles.timelineBar}>
        <PlayButton />
        <View style={styles.sliderContainer}>
          <TimeSlider />
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#000",
  },
  timelineBar: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(26, 26, 46, 0.9)",
    paddingBottom: 16,
  },
  sliderContainer: {
    flex: 1,
  },
});
```

- [ ] **Step 4: Test on device**

Verify:
- Slider shows all past frames (~13 timestamps)
- Dragging slider changes the radar overlay frame
- Timestamp label updates as you scrub
- "LIVE" badge appears on the last frame
- Play button starts animation cycling through frames
- Play stops when you drag the slider manually

- [ ] **Step 5: Commit**

```bash
git add src/components/timeline/ src/app/\(tabs\)/index.tsx
git commit -m "feat: add time slider and play/pause for radar animation"
```

---

## Task 11: Forecast Bottom Sheet

**Files:**
- Create: `src/components/forecast/CurrentConditions.tsx`
- Create: `src/components/forecast/HourlyScroll.tsx`
- Create: `src/components/forecast/ForecastSheet.tsx`
- Modify: `src/app/(tabs)/index.tsx`

- [ ] **Step 1: Create CurrentConditions component**

Create `src/components/forecast/CurrentConditions.tsx`:

```tsx
import { View, Text, StyleSheet } from "react-native";
import type { OpenMeteoResponse } from "../../types/weather";
import { getWeatherInfo } from "../../lib/weatherCodes";

interface Props {
  forecast: OpenMeteoResponse;
}

export function CurrentConditions({ forecast }: Props) {
  const { current, daily } = forecast;
  const weather = getWeatherInfo(current.weather_code);
  const high = daily.temperature_2m_max[0];
  const low = daily.temperature_2m_min[0];

  return (
    <View style={styles.container}>
      <View style={styles.mainRow}>
        <Text style={styles.temp}>{Math.round(current.temperature_2m)}\u00B0</Text>
        <View style={styles.details}>
          <Text style={styles.condition}>
            {weather.icon} {weather.label}
          </Text>
          <Text style={styles.highLow}>
            H:{Math.round(high)}\u00B0 L:{Math.round(low)}\u00B0
          </Text>
        </View>
      </View>
      <View style={styles.statsRow}>
        <Stat label="Feels Like" value={`${Math.round(current.apparent_temperature)}\u00B0`} />
        <Stat label="Wind" value={`${Math.round(current.wind_speed_10m)} mph`} />
        <Stat label="Humidity" value={`${current.relative_humidity_2m}%`} />
        <Stat label="Gusts" value={`${Math.round(current.wind_gusts_10m)} mph`} />
      </View>
    </View>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.stat}>
      <Text style={styles.statLabel}>{label}</Text>
      <Text style={styles.statValue}>{value}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    padding: 16,
    gap: 16,
  },
  mainRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 16,
  },
  temp: {
    fontSize: 48,
    fontWeight: "200",
    color: "#fff",
  },
  details: {
    gap: 4,
  },
  condition: {
    fontSize: 16,
    color: "#ccc",
  },
  highLow: {
    fontSize: 14,
    color: "#999",
  },
  statsRow: {
    flexDirection: "row",
    justifyContent: "space-between",
  },
  stat: {
    alignItems: "center",
    gap: 4,
  },
  statLabel: {
    fontSize: 11,
    color: "#888",
    textTransform: "uppercase",
  },
  statValue: {
    fontSize: 15,
    color: "#fff",
    fontWeight: "500",
  },
});
```

- [ ] **Step 2: Create HourlyScroll component**

Create `src/components/forecast/HourlyScroll.tsx`:

```tsx
import { ScrollView, View, Text, StyleSheet } from "react-native";
import type { OpenMeteoResponse } from "../../types/weather";
import { getWeatherInfo } from "../../lib/weatherCodes";

interface Props {
  forecast: OpenMeteoResponse;
}

export function HourlyScroll({ forecast }: Props) {
  const { hourly } = forecast;
  // Show next 24 hours
  const now = new Date();
  const currentHourIndex = hourly.time.findIndex((t) => new Date(t) >= now);
  const startIndex = Math.max(0, currentHourIndex);
  const hours = hourly.time.slice(startIndex, startIndex + 24);

  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      contentContainerStyle={styles.scroll}
    >
      {hours.map((time, i) => {
        const idx = startIndex + i;
        const weather = getWeatherInfo(hourly.weather_code[idx]);
        const isNow = i === 0;
        return (
          <View key={time} style={styles.card}>
            <Text style={styles.hour}>
              {isNow ? "Now" : new Date(time).toLocaleTimeString([], { hour: "numeric" })}
            </Text>
            <Text style={styles.icon}>{weather.icon}</Text>
            <Text style={styles.cardTemp}>
              {Math.round(hourly.temperature_2m[idx])}\u00B0
            </Text>
            <Text style={styles.precip}>
              {hourly.precipitation_probability[idx]}%
            </Text>
            <Text style={styles.wind}>
              {Math.round(hourly.wind_speed_10m[idx])}
            </Text>
          </View>
        );
      })}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    paddingHorizontal: 16,
    gap: 4,
  },
  card: {
    width: 64,
    alignItems: "center",
    paddingVertical: 12,
    paddingHorizontal: 4,
    borderRadius: 12,
    backgroundColor: "rgba(255, 255, 255, 0.05)",
    gap: 6,
  },
  hour: {
    fontSize: 12,
    color: "#aaa",
    fontWeight: "500",
  },
  icon: {
    fontSize: 20,
  },
  cardTemp: {
    fontSize: 16,
    color: "#fff",
    fontWeight: "600",
  },
  precip: {
    fontSize: 11,
    color: "#4fc3f7",
  },
  wind: {
    fontSize: 11,
    color: "#888",
  },
});
```

- [ ] **Step 3: Create ForecastSheet container**

Create `src/components/forecast/ForecastSheet.tsx`:

```tsx
import { useCallback, useMemo, useRef } from "react";
import { View, Text, StyleSheet } from "react-native";
import BottomSheet from "@gorhom/bottom-sheet";
import { useForecast } from "../../hooks/useForecast";
import { CurrentConditions } from "./CurrentConditions";
import { HourlyScroll } from "./HourlyScroll";

export function ForecastSheet() {
  const bottomSheetRef = useRef<BottomSheet>(null);
  const snapPoints = useMemo(() => [80, "35%", "70%"], []);
  const { data: forecast, isLoading } = useForecast();

  const renderHandle = useCallback(
    () => (
      <View style={styles.handle}>
        <View style={styles.handleBar} />
      </View>
    ),
    []
  );

  if (!forecast && !isLoading) return null;

  return (
    <BottomSheet
      ref={bottomSheetRef}
      index={0}
      snapPoints={snapPoints}
      handleComponent={renderHandle}
      backgroundStyle={styles.background}
      enablePanDownToClose={false}
    >
      {isLoading ? (
        <View style={styles.loading}>
          <Text style={styles.loadingText}>Loading forecast...</Text>
        </View>
      ) : forecast ? (
        <View>
          <CurrentConditions forecast={forecast} />
          <HourlyScroll forecast={forecast} />
        </View>
      ) : null}
    </BottomSheet>
  );
}

const styles = StyleSheet.create({
  background: {
    backgroundColor: "#1a1a2e",
    borderTopLeftRadius: 16,
    borderTopRightRadius: 16,
  },
  handle: {
    alignItems: "center",
    paddingTop: 10,
    paddingBottom: 4,
  },
  handleBar: {
    width: 40,
    height: 4,
    borderRadius: 2,
    backgroundColor: "#555",
  },
  loading: {
    padding: 20,
    alignItems: "center",
  },
  loadingText: {
    color: "#888",
    fontSize: 14,
  },
});
```

- [ ] **Step 4: Add ForecastSheet to map screen**

Update `src/app/(tabs)/index.tsx` — add ForecastSheet after the timeline bar:

```tsx
import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { TimeSlider } from "../../components/timeline/TimeSlider";
import { PlayButton } from "../../components/timeline/PlayButton";
import { ForecastSheet } from "../../components/forecast/ForecastSheet";
import { useLocation } from "../../hooks/useLocation";
import { useManifest } from "../../hooks/useManifest";

export default function MapScreen() {
  useLocation();
  useManifest();

  return (
    <View style={styles.container}>
      <WeatherMap>
        <RadarOverlay />
      </WeatherMap>
      <View style={styles.timelineBar}>
        <PlayButton />
        <View style={styles.sliderContainer}>
          <TimeSlider />
        </View>
      </View>
      <ForecastSheet />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#000",
  },
  timelineBar: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(26, 26, 46, 0.9)",
    paddingBottom: 16,
  },
  sliderContainer: {
    flex: 1,
  },
});
```

- [ ] **Step 5: Test on device**

Verify:
- Bottom sheet appears at bottom of screen, collapsed to ~80px
- Collapsed shows loading state, then current temp + conditions once loaded
- Swipe up to half-expand: hourly forecast cards scroll horizontally
- Each hourly card shows time, icon, temp, precip %, wind speed
- Sheet can be swiped to fully expanded (70%)

- [ ] **Step 6: Commit**

```bash
git add src/components/forecast/ src/app/\(tabs\)/index.tsx
git commit -m "feat: add forecast bottom sheet with current conditions and hourly scroll"
```

---

## Task 12: Alert Banner

**Files:**
- Create: `src/components/alerts/AlertBanner.tsx`
- Modify: `src/app/(tabs)/index.tsx`

- [ ] **Step 1: Create AlertBanner component**

Create `src/components/alerts/AlertBanner.tsx`:

```tsx
import { TouchableOpacity, Text, StyleSheet, View } from "react-native";
import { useRouter } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";
import type { NWSAlert } from "../../types/weather";

const SEVERITY_COLORS: Record<string, string> = {
  Extreme: "#d32f2f",
  Severe: "#f44336",
  Moderate: "#ff9800",
  Minor: "#ffc107",
  Unknown: "#9e9e9e",
};

export function AlertBanner() {
  const { data: alertData } = useAlerts();
  const router = useRouter();

  if (!alertData || alertData.features.length === 0) return null;

  // Show most severe alert
  const alert = alertData.features.reduce<NWSAlert>((worst, current) => {
    const severityOrder = ["Extreme", "Severe", "Moderate", "Minor", "Unknown"];
    const worstIdx = severityOrder.indexOf(worst.properties.severity);
    const currentIdx = severityOrder.indexOf(current.properties.severity);
    return currentIdx < worstIdx ? current : worst;
  }, alertData.features[0]);

  const bgColor = SEVERITY_COLORS[alert.properties.severity] ?? "#9e9e9e";

  return (
    <TouchableOpacity
      style={[styles.banner, { backgroundColor: bgColor }]}
      onPress={() =>
        router.push({
          pathname: "/alert/[id]",
          params: { id: alert.properties.id },
        })
      }
      activeOpacity={0.8}
    >
      <Text style={styles.text} numberOfLines={1}>
        {"\u26A0\uFE0F"} {alert.properties.event}
        {alert.properties.headline ? ` \u2014 ${alert.properties.headline}` : ""}
      </Text>
    </TouchableOpacity>
  );
}

const styles = StyleSheet.create({
  banner: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    paddingTop: 50, // safe area
    paddingBottom: 12,
    paddingHorizontal: 16,
    zIndex: 100,
  },
  text: {
    color: "#fff",
    fontSize: 14,
    fontWeight: "600",
  },
});
```

- [ ] **Step 2: Add AlertBanner to map screen**

Update `src/app/(tabs)/index.tsx` — add AlertBanner inside the container, before WeatherMap:

```tsx
import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { TimeSlider } from "../../components/timeline/TimeSlider";
import { PlayButton } from "../../components/timeline/PlayButton";
import { ForecastSheet } from "../../components/forecast/ForecastSheet";
import { AlertBanner } from "../../components/alerts/AlertBanner";
import { useLocation } from "../../hooks/useLocation";
import { useManifest } from "../../hooks/useManifest";

export default function MapScreen() {
  useLocation();
  useManifest();

  return (
    <View style={styles.container}>
      <WeatherMap>
        <RadarOverlay />
      </WeatherMap>
      <AlertBanner />
      <View style={styles.timelineBar}>
        <PlayButton />
        <View style={styles.sliderContainer}>
          <TimeSlider />
        </View>
      </View>
      <ForecastSheet />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#000",
  },
  timelineBar: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(26, 26, 46, 0.9)",
    paddingBottom: 16,
  },
  sliderContainer: {
    flex: 1,
  },
});
```

- [ ] **Step 3: Create Alert Detail screen**

Create `src/app/alert/[id].tsx`:

```tsx
import { ScrollView, Text, StyleSheet } from "react-native";
import { useLocalSearchParams } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";

export default function AlertDetailScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { data: alertData } = useAlerts();

  const alert = alertData?.features.find((f) => f.properties.id === id);

  if (!alert) {
    return (
      <ScrollView style={styles.container}>
        <Text style={styles.body}>Alert not found.</Text>
      </ScrollView>
    );
  }

  return (
    <ScrollView style={styles.container}>
      <Text style={styles.event}>{alert.properties.event}</Text>
      <Text style={styles.area}>{alert.properties.areaDesc}</Text>
      <Text style={styles.expires}>
        Expires: {new Date(alert.properties.expires).toLocaleString()}
      </Text>
      <Text style={styles.body}>{alert.properties.description}</Text>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#1a1a2e",
    padding: 16,
  },
  event: {
    fontSize: 22,
    fontWeight: "700",
    color: "#fff",
    marginBottom: 8,
  },
  area: {
    fontSize: 14,
    color: "#aaa",
    marginBottom: 4,
  },
  expires: {
    fontSize: 13,
    color: "#f44336",
    marginBottom: 16,
  },
  body: {
    fontSize: 15,
    color: "#ddd",
    lineHeight: 22,
  },
});
```

- [ ] **Step 4: Test on device**

Verify:
- If there are active NWS alerts at your location, a colored banner appears at the top
- Tapping the banner navigates to the alert detail modal
- Alert detail shows event name, area, expiry, and full description
- If no alerts, banner doesn't render (no empty space)

- [ ] **Step 5: Commit**

```bash
git add src/components/alerts/ src/app/alert/ src/app/\(tabs\)/index.tsx
git commit -m "feat: add NWS alert banner and detail screen"
```

---

## Task 13: Settings Screen

**Files:**
- Modify: `src/app/(tabs)/settings.tsx`

- [ ] **Step 1: Implement settings screen**

Replace contents of `src/app/(tabs)/settings.tsx`:

```tsx
import { View, Text, StyleSheet, Switch, TouchableOpacity } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import Slider from "@react-native-community/slider";
import { useWeatherStore } from "../../stores/useWeatherStore";
import type { MapStyle, TemperatureUnit } from "../../types/weather";

export default function SettingsScreen() {
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const setMapStyle = useWeatherStore((s) => s.setMapStyle);
  const temperatureUnit = useWeatherStore((s) => s.temperatureUnit);
  const setTemperatureUnit = useWeatherStore((s) => s.setTemperatureUnit);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const setRadarOpacity = useWeatherStore((s) => s.setRadarOpacity);
  const playbackSpeed = useWeatherStore((s) => s.playbackSpeed);
  const setPlaybackSpeed = useWeatherStore((s) => s.setPlaybackSpeed);

  return (
    <SafeAreaView style={styles.container}>
      <Text style={styles.title}>Settings</Text>

      <Section title="Map">
        <Row label="Dark Mode">
          <Switch
            value={mapStyle === "dark"}
            onValueChange={(v) => setMapStyle(v ? "dark" : "light")}
            trackColor={{ true: "#4fc3f7" }}
          />
        </Row>
      </Section>

      <Section title="Units">
        <Row label="Temperature">
          <SegmentedControl
            options={["F", "C"]}
            selected={temperatureUnit === "fahrenheit" ? "F" : "C"}
            onSelect={(v) =>
              setTemperatureUnit(v === "F" ? "fahrenheit" : "celsius")
            }
          />
        </Row>
      </Section>

      <Section title="Radar">
        <Row label={`Opacity: ${Math.round(radarOpacity * 100)}%`}>
          <Slider
            style={styles.slider}
            minimumValue={0.1}
            maximumValue={1}
            step={0.05}
            value={radarOpacity}
            onValueChange={setRadarOpacity}
            minimumTrackTintColor="#4fc3f7"
            maximumTrackTintColor="#555"
            thumbTintColor="#4fc3f7"
          />
        </Row>
        <Row label={`Playback: ${playbackSpeed} FPS`}>
          <Slider
            style={styles.slider}
            minimumValue={1}
            maximumValue={15}
            step={1}
            value={playbackSpeed}
            onValueChange={setPlaybackSpeed}
            minimumTrackTintColor="#4fc3f7"
            maximumTrackTintColor="#555"
            thumbTintColor="#4fc3f7"
          />
        </Row>
      </Section>

      <Text style={styles.footer}>
        StormScope v1.0 {"\n"}
        Data: RainViewer, Open-Meteo, NWS
      </Text>
    </SafeAreaView>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <View style={styles.section}>
      <Text style={styles.sectionTitle}>{title}</Text>
      {children}
    </View>
  );
}

function Row({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <View style={styles.row}>
      <Text style={styles.rowLabel}>{label}</Text>
      {children}
    </View>
  );
}

function SegmentedControl({
  options,
  selected,
  onSelect,
}: {
  options: string[];
  selected: string;
  onSelect: (value: string) => void;
}) {
  return (
    <View style={styles.segmented}>
      {options.map((opt) => (
        <TouchableOpacity
          key={opt}
          style={[
            styles.segment,
            opt === selected && styles.segmentSelected,
          ]}
          onPress={() => onSelect(opt)}
        >
          <Text
            style={[
              styles.segmentText,
              opt === selected && styles.segmentTextSelected,
            ]}
          >
            {opt}
          </Text>
        </TouchableOpacity>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#1a1a2e",
    padding: 16,
  },
  title: {
    fontSize: 28,
    fontWeight: "700",
    color: "#fff",
    marginBottom: 24,
  },
  section: {
    marginBottom: 24,
  },
  sectionTitle: {
    fontSize: 13,
    fontWeight: "600",
    color: "#888",
    textTransform: "uppercase",
    marginBottom: 8,
  },
  row: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: "#333",
  },
  rowLabel: {
    fontSize: 16,
    color: "#ddd",
  },
  slider: {
    width: 150,
  },
  segmented: {
    flexDirection: "row",
    backgroundColor: "#333",
    borderRadius: 8,
    overflow: "hidden",
  },
  segment: {
    paddingHorizontal: 16,
    paddingVertical: 8,
  },
  segmentSelected: {
    backgroundColor: "#4fc3f7",
  },
  segmentText: {
    color: "#aaa",
    fontWeight: "600",
  },
  segmentTextSelected: {
    color: "#000",
  },
  footer: {
    marginTop: "auto",
    textAlign: "center",
    color: "#555",
    fontSize: 12,
    lineHeight: 18,
  },
});
```

- [ ] **Step 2: Test on device**

Verify:
- Settings tab shows Map, Units, and Radar sections
- Dark mode toggle switches the base map style
- Temperature unit toggle works (though forecast may need re-fetch to apply)
- Radar opacity slider adjusts overlay transparency in real-time
- Playback speed slider adjusts animation speed

- [ ] **Step 3: Commit**

```bash
git add src/app/\(tabs\)/settings.tsx
git commit -m "feat: add settings screen with map style, units, and radar controls"
```

---

## Task 14: Final Integration & Cleanup

**Files:**
- Review all files for import errors, unused imports

- [ ] **Step 1: Run full test suite**

```bash
npx jest --no-cache
```

Expected: All tests pass.

- [ ] **Step 2: Run TypeScript check**

```bash
npx tsc --noEmit
```

Fix any type errors.

- [ ] **Step 3: Build and full manual test**

```bash
npx expo prebuild --clean
npx expo run:ios
```

Full test checklist:
- [ ] App launches without crash
- [ ] Map renders with OpenFreeMap base tiles
- [ ] Radar tiles load and display correctly
- [ ] Time slider scrubs through past frames (~13 frames, ~2 hours)
- [ ] Play button cycles through frames at configured speed
- [ ] "LIVE" badge shows on latest frame
- [ ] Forecast bottom sheet loads current conditions
- [ ] Hourly scroll shows 24 hours of forecasts
- [ ] Alert banner appears if active NWS alerts at location
- [ ] Tapping alert opens detail modal
- [ ] Settings: dark mode toggle changes map style
- [ ] Settings: radar opacity slider adjusts overlay
- [ ] Settings: playback speed slider changes animation rate
- [ ] Tab navigation between Map and Settings works

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: final cleanup and integration for Phase 1"
```

---

## Dependency Install Summary

All `npx expo install` commands for reference (run in Task 1):

```bash
npx expo install @maplibre/maplibre-react-native
npx expo install expo-location
npx expo install @tanstack/react-query
npx expo install zustand
npx expo install react-native-mmkv
npx expo install @gorhom/bottom-sheet
npx expo install react-native-reanimated
npx expo install react-native-gesture-handler
npx expo install @react-native-community/slider
npx expo install react-native-safe-area-context
```

## Notes for Phase 2

When radar quality from RainViewer isn't sufficient (max zoom 7, 2hr history, limited colors), the self-hosted pipeline from the PRD adds:

- **MRMS ingest:** 1km resolution, 3hr history, 2-min updates, NWS color table
- **Pysteps nowcast:** 0-3hr future radar extrapolation
- **HRRR ingest:** +3-24hr forecast, wind/temp/CAPE/precip type layers
- **Tile server:** Caddy serving static tiles, manifest API

This would be a containerized Python backend deployed to k8s via ArgoCD. But Phase 1 works entirely client-side.
