# StormScope App Enhancements — Implementation Plan

> ⚠️ **Historical — executed before the 2026-05-02 Temporal refactor.** `src/...` paths below are now under `frontend/src/...`. Kept verbatim as a record of how this plan shipped.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enhance the React Native app to support the self-hosted tile server (with RainViewer fallback), multi-layer display with a layer picker, extended timeline (-3h to +24h), daily forecast, alert polygons on the map, and UI polish.

**Architecture:** Add a server URL setting stored in MMKV. Abstract manifest fetching to support both RainViewer and self-hosted formats. Add a layer store to Zustand managing which layers are visible. Each layer type gets its own RasterSource/RasterLayer in the map. The timeline slider handles variable-density timestamps across past/forecast data.

**Tech Stack:** Expo SDK 55, MapLibre React Native, Zustand, TanStack Query, react-native-mmkv, @gorhom/bottom-sheet

---

## File Structure (new/modified files only)

```
src/
├── lib/
│   ├── constants.ts                    # MODIFY: add layer definitions, self-hosted defaults
│   ├── api.ts                          # MODIFY: add fetchSelfHostedManifest, server URL support
│   ├── tileUrl.ts                      # MODIFY: add buildSelfHostedTileUrl
│   └── storage.ts                      # CREATE: MMKV wrapper for persistent settings
├── types/
│   └── weather.ts                      # MODIFY: add SelfHostedManifest, LayerType, LayerConfig types
├── stores/
│   └── useWeatherStore.ts              # MODIFY: add layer state, activeLayer, serverUrl, data source
├── hooks/
│   ├── useManifest.ts                  # MODIFY: switch between RainViewer and self-hosted based on setting
│   └── useServerHealth.ts              # CREATE: ping self-hosted server to check availability
├── components/
│   ├── map/
│   │   ├── RadarOverlay.tsx            # MODIFY: use active source (RainViewer or self-hosted)
│   │   ├── WeatherLayerOverlay.tsx     # CREATE: generic overlay for any self-hosted layer
│   │   └── AlertPolygon.tsx            # CREATE: render NWS alert geometry on map
│   ├── layers/
│   │   └── LayerPicker.tsx             # CREATE: FAB stack for layer selection
│   ├── timeline/
│   │   └── TimeSlider.tsx              # MODIFY: handle variable-density timestamps, forecast labels
│   ├── forecast/
│   │   ├── ForecastSheet.tsx           # MODIFY: add daily forecast section
│   │   └── DailyForecast.tsx           # CREATE: 7-day daily forecast
│   └── alerts/
│       └── AlertBanner.tsx             # (unchanged)
├── app/
│   ├── (tabs)/
│   │   ├── index.tsx                   # MODIFY: add LayerPicker, AlertPolygon, WeatherLayerOverlay
│   │   └── settings.tsx                # MODIFY: add server URL input, data source toggle
```

---

### Task 1: MMKV Storage & Types

**Files:**
- Create: `src/lib/storage.ts`
- Modify: `src/types/weather.ts`
- Modify: `src/lib/constants.ts`

- [ ] **Step 1: Create MMKV storage wrapper**

Create `src/lib/storage.ts`:

```typescript
import { MMKV } from "react-native-mmkv";

export const storage = new MMKV({ id: "stormscope" });

export function getString(key: string, fallback: string): string {
  return storage.getString(key) ?? fallback;
}

export function setString(key: string, value: string): void {
  storage.set(key, value);
}

export function getBoolean(key: string, fallback: boolean): boolean {
  const val = storage.getBoolean(key);
  return val !== undefined ? val : fallback;
}

export function setBoolean(key: string, value: boolean): void {
  storage.set(key, value);
}
```

- [ ] **Step 2: Add types for self-hosted manifest and layers**

Add to `src/types/weather.ts` after the existing types:

```typescript
// --- Self-Hosted Tile Server ---

export interface SelfHostedManifest {
  layers: Record<string, { timestamps: string[] }>;
  tile_url_template: string;
  updated_at: string;
}

// --- Layers ---

export type LayerType =
  | "radar"
  | "radar-hrrr"
  | "temperature"
  | "wind"
  | "cape"
  | "precip-type";

export type DataSource = "rainviewer" | "selfhosted";

export interface LayerConfig {
  id: LayerType;
  label: string;
  icon: string;
  isFillLayer: boolean; // mutually exclusive with other fill layers
  defaultVisible: boolean;
  minZoom: number;
  maxZoom: number;
}
```

- [ ] **Step 3: Add layer definitions and self-hosted defaults to constants**

Add to `src/lib/constants.ts`:

```typescript
import type { LayerConfig, LayerType } from "../types/weather";

// ... keep existing API, MAP_STYLES, RADAR, DEFAULTS ...

export const SELF_HOSTED = {
  DEFAULT_URL: "http://localhost:8080",
  MANIFEST_PATH: "/api/manifest.json",
  TILE_PATTERN: "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
  FORECAST_PATH: "/api/forecast",
} as const;

export const LAYERS: LayerConfig[] = [
  { id: "radar", label: "Radar", icon: "\uD83D\uDFE2", isFillLayer: true, defaultVisible: true, minZoom: 1, maxZoom: 12 },
  { id: "wind", label: "Wind", icon: "\uD83D\uDCA8", isFillLayer: false, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "temperature", label: "Temp", icon: "\uD83C\uDF21\uFE0F", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "precip-type", label: "Precip", icon: "\uD83C\uDF27\uFE0F", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "cape", label: "Severe", icon: "\u26A1", isFillLayer: false, defaultVisible: false, minZoom: 1, maxZoom: 9 },
];
```

- [ ] **Step 4: Commit**

```bash
git add src/lib/storage.ts src/types/weather.ts src/lib/constants.ts
git commit -m "feat: add MMKV storage, self-hosted types, and layer definitions"
```

---

### Task 2: Store Updates — Layers, Server URL, Data Source

**Files:**
- Modify: `src/stores/useWeatherStore.ts`

- [ ] **Step 1: Add layer and server state to store**

Replace `src/stores/useWeatherStore.ts`:

```typescript
import { create } from "zustand";
import type { RadarFrame, TemperatureUnit, MapStyle, LayerType, DataSource } from "../types/weather";
import { DEFAULTS, RADAR } from "../lib/constants";
import { getString, setString } from "../lib/storage";

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
  activeLayer: LayerType;
  visibleOverlays: Set<LayerType>;

  // Settings
  temperatureUnit: TemperatureUnit;
  mapStyle: MapStyle;
  dataSource: DataSource;
  serverUrl: string;

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
  setActiveLayer: (layer: LayerType) => void;
  toggleOverlay: (layer: LayerType) => void;
  setDataSource: (source: DataSource) => void;
  setServerUrl: (url: string) => void;
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
  activeLayer: "radar" as LayerType,
  visibleOverlays: new Set<LayerType>(),

  temperatureUnit: "fahrenheit",
  mapStyle: "light",
  dataSource: (getString("dataSource", "rainviewer") as DataSource),
  serverUrl: getString("serverUrl", "http://localhost:8080"),

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
  setActiveLayer: (layer) => set({ activeLayer: layer }),
  toggleOverlay: (layer) => set((s) => {
    const next = new Set(s.visibleOverlays);
    if (next.has(layer)) {
      next.delete(layer);
    } else {
      next.add(layer);
    }
    return { visibleOverlays: next };
  }),
  setDataSource: (source) => {
    setString("dataSource", source);
    set({ dataSource: source });
  },
  setServerUrl: (url) => {
    setString("serverUrl", url);
    set({ serverUrl: url });
  },
  nextFrame: () => {
    const { frames, currentFrameIndex } = get();
    if (frames.length === 0) return;
    set({ currentFrameIndex: (currentFrameIndex + 1) % frames.length });
  },
}));
```

- [ ] **Step 2: Update store tests**

Replace `__tests__/stores/useWeatherStore.test.ts`:

```typescript
import { useWeatherStore } from "../../src/stores/useWeatherStore";

// Mock MMKV
jest.mock("../../src/lib/storage", () => ({
  getString: jest.fn(() => "rainviewer"),
  setString: jest.fn(),
  getBoolean: jest.fn(() => false),
  setBoolean: jest.fn(),
}));

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
    expect(state.activeLayer).toBe("radar");
    expect(state.dataSource).toBe("rainviewer");
  });

  it("setFrames updates frames", () => {
    const frames = [{ time: 1000, path: "/a" }, { time: 2000, path: "/b" }];
    useWeatherStore.getState().setFrames(frames);
    expect(useWeatherStore.getState().frames).toEqual(frames);
  });

  it("nextFrame wraps around to 0", () => {
    useWeatherStore.setState({
      frames: [{ time: 1, path: "/a" }, { time: 2, path: "/b" }],
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
  });

  it("setLocation updates lat/lon", () => {
    useWeatherStore.getState().setLocation(38.9, -77.0);
    const { latitude, longitude } = useWeatherStore.getState();
    expect(latitude).toBe(38.9);
    expect(longitude).toBe(-77.0);
  });

  it("setActiveLayer changes active layer", () => {
    useWeatherStore.getState().setActiveLayer("temperature");
    expect(useWeatherStore.getState().activeLayer).toBe("temperature");
  });

  it("toggleOverlay adds and removes overlays", () => {
    useWeatherStore.getState().toggleOverlay("wind");
    expect(useWeatherStore.getState().visibleOverlays.has("wind")).toBe(true);
    useWeatherStore.getState().toggleOverlay("wind");
    expect(useWeatherStore.getState().visibleOverlays.has("wind")).toBe(false);
  });
});
```

- [ ] **Step 3: Run tests**

```bash
npx jest --no-cache
```

Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/stores/useWeatherStore.ts __tests__/stores/useWeatherStore.test.ts
git commit -m "feat: add layer state, server URL, and data source to store"
```

---

### Task 3: API Updates — Self-Hosted Manifest & Tile URLs

**Files:**
- Modify: `src/lib/api.ts`
- Modify: `src/lib/tileUrl.ts`
- Modify: `src/hooks/useManifest.ts`

- [ ] **Step 1: Add self-hosted API functions**

Add to `src/lib/api.ts`:

```typescript
import { API } from "./constants";
import type {
  RainViewerManifest,
  OpenMeteoResponse,
  NWSAlertCollection,
  SelfHostedManifest,
} from "../types/weather";

// ... keep existing fetchRadarManifest, fetchForecast, fetchAlerts ...

export async function fetchSelfHostedManifest(
  serverUrl: string
): Promise<SelfHostedManifest> {
  const res = await fetch(`${serverUrl}/api/manifest.json`);
  if (!res.ok) throw new Error(`Tile server error: ${res.status}`);
  return res.json();
}

export async function checkServerHealth(serverUrl: string): Promise<boolean> {
  try {
    const res = await fetch(`${serverUrl}/api/health`, { signal: AbortSignal.timeout(5000) });
    return res.ok;
  } catch {
    return false;
  }
}
```

- [ ] **Step 2: Add self-hosted tile URL builder**

Add to `src/lib/tileUrl.ts`:

```typescript
import type { RadarFrame, LayerType } from "../types/weather";
import { RADAR } from "./constants";

// ... keep existing buildRadarTileUrl ...

export function buildSelfHostedTileUrl(
  serverUrl: string,
  layer: LayerType,
  timestamp: string
): string {
  return `${serverUrl}/tiles/${layer}/${timestamp}/{z}/{x}/{y}.png`;
}
```

- [ ] **Step 3: Update useManifest to support both sources**

Replace `src/hooks/useManifest.ts`:

```typescript
import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchRadarManifest, fetchSelfHostedManifest } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";
import type { RadarFrame } from "../types/weather";

export function useManifest() {
  const setFrames = useWeatherStore((s) => s.setFrames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);

  const rainviewerQuery = useQuery({
    queryKey: ["radar-manifest-rainviewer"],
    queryFn: fetchRadarManifest,
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
    enabled: dataSource === "rainviewer",
  });

  const selfHostedQuery = useQuery({
    queryKey: ["manifest-selfhosted", serverUrl],
    queryFn: () => fetchSelfHostedManifest(serverUrl),
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
    enabled: dataSource === "selfhosted",
  });

  useEffect(() => {
    let allFrames: RadarFrame[] = [];

    if (dataSource === "rainviewer" && rainviewerQuery.data) {
      allFrames = [
        ...rainviewerQuery.data.radar.past,
        ...rainviewerQuery.data.radar.nowcast,
      ];
    } else if (dataSource === "selfhosted" && selfHostedQuery.data) {
      // Convert self-hosted timestamps to RadarFrame format
      const layerKey = activeLayer === "radar" ? "radar" : activeLayer;
      const layerData = selfHostedQuery.data.layers[layerKey];
      if (layerData) {
        allFrames = layerData.timestamps.map((ts) => ({
          time: Math.floor(new Date(ts).getTime() / 1000),
          path: ts, // Store the ISO timestamp as path for self-hosted
        }));
      }
    }

    if (allFrames.length > 0) {
      setFrames(allFrames);
      if (currentFrameIndex === -1 || currentFrameIndex >= allFrames.length) {
        setCurrentFrameIndex(allFrames.length - 1);
      }
    }
  }, [rainviewerQuery.data, selfHostedQuery.data, dataSource, activeLayer]);

  return dataSource === "rainviewer" ? rainviewerQuery : selfHostedQuery;
}
```

- [ ] **Step 4: Update API tests**

Add to `__tests__/lib/api.test.ts`:

```typescript
// Add after existing tests:

describe("fetchSelfHostedManifest", () => {
  it("fetches manifest from server URL", async () => {
    const { fetchSelfHostedManifest } = require("../../src/lib/api");
    const manifest = {
      layers: { radar: { timestamps: ["2026-04-14T18:00:00Z"] } },
      tile_url_template: "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
      updated_at: "2026-04-14T18:04:00Z",
    };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(manifest),
    });

    const result = await fetchSelfHostedManifest("http://localhost:8080");
    expect(result).toEqual(manifest);
    expect(mockFetch).toHaveBeenCalledWith("http://localhost:8080/api/manifest.json");
  });
});

describe("checkServerHealth", () => {
  it("returns true when server is healthy", async () => {
    const { checkServerHealth } = require("../../src/lib/api");
    mockFetch.mockResolvedValueOnce({ ok: true });
    const result = await checkServerHealth("http://localhost:8080");
    expect(result).toBe(true);
  });

  it("returns false when server is unreachable", async () => {
    const { checkServerHealth } = require("../../src/lib/api");
    mockFetch.mockRejectedValueOnce(new Error("Network error"));
    const result = await checkServerHealth("http://localhost:8080");
    expect(result).toBe(false);
  });
});
```

- [ ] **Step 5: Run tests and commit**

```bash
npx jest --no-cache
git add src/lib/api.ts src/lib/tileUrl.ts src/hooks/useManifest.ts __tests__/lib/api.test.ts
git commit -m "feat: add self-hosted manifest support with RainViewer fallback"
```

---

### Task 4: Layer Picker Component

**Files:**
- Create: `src/components/layers/LayerPicker.tsx`

- [ ] **Step 1: Create LayerPicker**

Create `src/components/layers/LayerPicker.tsx`:

```tsx
import { View, TouchableOpacity, Text, StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { LAYERS } from "../../lib/constants";
import type { LayerConfig, LayerType } from "../../types/weather";

export function LayerPicker() {
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const setActiveLayer = useWeatherStore((s) => s.setActiveLayer);
  const visibleOverlays = useWeatherStore((s) => s.visibleOverlays);
  const toggleOverlay = useWeatherStore((s) => s.toggleOverlay);
  const dataSource = useWeatherStore((s) => s.dataSource);

  const handlePress = (layer: LayerConfig) => {
    if (layer.isFillLayer) {
      setActiveLayer(layer.id);
    } else {
      toggleOverlay(layer.id);
    }
  };

  const isActive = (layer: LayerConfig) => {
    if (layer.isFillLayer) return activeLayer === layer.id;
    return visibleOverlays.has(layer.id);
  };

  // Only show non-radar layers when using self-hosted
  const availableLayers =
    dataSource === "rainviewer"
      ? LAYERS.filter((l) => l.id === "radar")
      : LAYERS;

  return (
    <View style={styles.container}>
      {availableLayers.map((layer) => (
        <TouchableOpacity
          key={layer.id}
          style={[styles.button, isActive(layer) && styles.buttonActive]}
          onPress={() => handlePress(layer)}
          activeOpacity={0.7}
        >
          <Text style={styles.icon}>{layer.icon}</Text>
          <Text
            style={[styles.label, isActive(layer) && styles.labelActive]}
          >
            {layer.label}
          </Text>
        </TouchableOpacity>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    position: "absolute",
    right: 12,
    top: 100,
    gap: 8,
    zIndex: 50,
  },
  button: {
    width: 56,
    height: 56,
    borderRadius: 28,
    backgroundColor: "rgba(26, 26, 46, 0.85)",
    justifyContent: "center",
    alignItems: "center",
    borderWidth: 2,
    borderColor: "transparent",
  },
  buttonActive: {
    borderColor: "#4fc3f7",
    backgroundColor: "rgba(79, 195, 247, 0.15)",
  },
  icon: {
    fontSize: 18,
  },
  label: {
    fontSize: 9,
    color: "#888",
    marginTop: 1,
  },
  labelActive: {
    color: "#4fc3f7",
  },
});
```

- [ ] **Step 2: Commit**

```bash
git add src/components/layers/LayerPicker.tsx
git commit -m "feat: add layer picker FAB stack component"
```

---

### Task 5: Weather Layer Overlay & Updated Radar Overlay

**Files:**
- Create: `src/components/map/WeatherLayerOverlay.tsx`
- Modify: `src/components/map/RadarOverlay.tsx`

- [ ] **Step 1: Create generic WeatherLayerOverlay for self-hosted layers**

Create `src/components/map/WeatherLayerOverlay.tsx`:

```tsx
import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import type { LayerType } from "../../types/weather";
import { LAYERS } from "../../lib/constants";

interface Props {
  layerId: LayerType;
  opacity?: number;
}

export function WeatherLayerOverlay({ layerId, opacity = 0.7 }: Props) {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  if (frames.length === 0 || currentFrameIndex < 0) return null;

  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  const layerConfig = LAYERS.find((l) => l.id === layerId);
  if (!layerConfig) return null;

  // frame.path stores the ISO timestamp for self-hosted
  const tileUrl = buildSelfHostedTileUrl(serverUrl, layerId, frame.path);

  return (
    <MapLibreGL.RasterSource
      id={`${layerId}-source`}
      key={`${layerId}-${frame.path}`}
      tileUrlTemplates={[tileUrl]}
      tileSize={256}
      minZoomLevel={layerConfig.minZoom}
      maxZoomLevel={layerConfig.maxZoom}
    >
      <MapLibreGL.RasterLayer
        id={`${layerId}-layer`}
        style={{
          rasterOpacity: opacity,
          rasterFadeDuration: 0,
        }}
      />
    </MapLibreGL.RasterSource>
  );
}
```

- [ ] **Step 2: Update RadarOverlay to support both data sources**

Replace `src/components/map/RadarOverlay.tsx`:

```tsx
import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useManifest } from "../../hooks/useManifest";
import { buildRadarTileUrl, buildSelfHostedTileUrl } from "../../lib/tileUrl";
import { RADAR } from "../../lib/constants";

export function RadarOverlay() {
  const { data: manifest } = useManifest();
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);

  // Only render when radar is the active fill layer
  if (activeLayer !== "radar") return null;
  if (frames.length === 0 || currentFrameIndex < 0) return null;

  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  let tileUrl: string;
  let minZoom = RADAR.MIN_ZOOM;
  let maxZoom = RADAR.MAX_ZOOM;

  if (dataSource === "rainviewer" && manifest && "host" in manifest) {
    tileUrl = buildRadarTileUrl(manifest.host, frame);
  } else if (dataSource === "selfhosted") {
    tileUrl = buildSelfHostedTileUrl(serverUrl, "radar", frame.path);
    maxZoom = 12; // Self-hosted supports higher zoom
  } else {
    return null;
  }

  return (
    <MapLibreGL.RasterSource
      id="radar-source"
      key={frame.path}
      tileUrlTemplates={[tileUrl]}
      tileSize={RADAR.TILE_SIZE}
      minZoomLevel={minZoom}
      maxZoomLevel={maxZoom}
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

- [ ] **Step 3: Commit**

```bash
git add src/components/map/WeatherLayerOverlay.tsx src/components/map/RadarOverlay.tsx
git commit -m "feat: add self-hosted weather layer overlay and update radar for dual source"
```

---

### Task 6: Alert Polygon on Map

**Files:**
- Create: `src/components/map/AlertPolygon.tsx`

- [ ] **Step 1: Create AlertPolygon**

Create `src/components/map/AlertPolygon.tsx`:

```tsx
import MapLibreGL from "@maplibre/maplibre-react-native";
import { useAlerts } from "../../hooks/useAlerts";

const SEVERITY_FILL: Record<string, string> = {
  Extreme: "rgba(211, 47, 47, 0.25)",
  Severe: "rgba(244, 67, 54, 0.2)",
  Moderate: "rgba(255, 152, 0, 0.2)",
  Minor: "rgba(255, 193, 7, 0.15)",
  Unknown: "rgba(158, 158, 158, 0.1)",
};

const SEVERITY_STROKE: Record<string, string> = {
  Extreme: "#d32f2f",
  Severe: "#f44336",
  Moderate: "#ff9800",
  Minor: "#ffc107",
  Unknown: "#9e9e9e",
};

export function AlertPolygon() {
  const { data: alertData } = useAlerts();

  if (!alertData || alertData.features.length === 0) return null;

  // Filter to alerts with geometry
  const alertsWithGeometry = alertData.features.filter((f) => f.geometry !== null);
  if (alertsWithGeometry.length === 0) return null;

  const geojson: GeoJSON.FeatureCollection = {
    type: "FeatureCollection",
    features: alertsWithGeometry.map((alert) => ({
      type: "Feature",
      geometry: alert.geometry!,
      properties: {
        severity: alert.properties.severity,
        event: alert.properties.event,
      },
    })),
  };

  return (
    <MapLibreGL.ShapeSource id="alert-polygons" shape={geojson}>
      <MapLibreGL.FillLayer
        id="alert-fill"
        style={{
          fillColor: [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_FILL.Extreme,
            "Severe", SEVERITY_FILL.Severe,
            "Moderate", SEVERITY_FILL.Moderate,
            "Minor", SEVERITY_FILL.Minor,
            SEVERITY_FILL.Unknown,
          ],
        }}
      />
      <MapLibreGL.LineLayer
        id="alert-outline"
        style={{
          lineColor: [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_STROKE.Extreme,
            "Severe", SEVERITY_STROKE.Severe,
            "Moderate", SEVERITY_STROKE.Moderate,
            "Minor", SEVERITY_STROKE.Minor,
            SEVERITY_STROKE.Unknown,
          ],
          lineWidth: 2,
        }}
      />
    </MapLibreGL.ShapeSource>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add src/components/map/AlertPolygon.tsx
git commit -m "feat: add NWS alert polygon overlay on map"
```

---

### Task 7: Daily Forecast Component

**Files:**
- Create: `src/components/forecast/DailyForecast.tsx`
- Modify: `src/components/forecast/ForecastSheet.tsx`

- [ ] **Step 1: Create DailyForecast**

Create `src/components/forecast/DailyForecast.tsx`:

```tsx
import { View, Text, StyleSheet } from "react-native";
import type { OpenMeteoResponse } from "../../types/weather";
import { getWeatherInfo } from "../../lib/weatherCodes";

interface Props {
  forecast: OpenMeteoResponse;
}

export function DailyForecast({ forecast }: Props) {
  const { daily } = forecast;

  return (
    <View style={styles.container}>
      <Text style={styles.title}>7-Day Forecast</Text>
      {daily.time.map((date, i) => {
        const weather = getWeatherInfo(daily.weather_code[i]);
        const dayName =
          i === 0
            ? "Today"
            : new Date(date).toLocaleDateString([], { weekday: "short" });
        return (
          <View key={date} style={styles.row}>
            <Text style={styles.day}>{dayName}</Text>
            <Text style={styles.icon}>{weather.icon}</Text>
            <View style={styles.tempBar}>
              <Text style={styles.low}>{Math.round(daily.temperature_2m_min[i])}{"\u00B0"}</Text>
              <View style={styles.bar} />
              <Text style={styles.high}>{Math.round(daily.temperature_2m_max[i])}{"\u00B0"}</Text>
            </View>
            <Text style={styles.precip}>
              {daily.precipitation_sum[i] > 0
                ? `${daily.precipitation_sum[i].toFixed(1)}"`
                : ""}
            </Text>
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    padding: 16,
    gap: 4,
  },
  title: {
    fontSize: 14,
    fontWeight: "600",
    color: "#888",
    textTransform: "uppercase",
    marginBottom: 8,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: "#333",
  },
  day: {
    width: 50,
    fontSize: 15,
    color: "#ddd",
    fontWeight: "500",
  },
  icon: {
    width: 30,
    fontSize: 18,
    textAlign: "center",
  },
  tempBar: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    marginHorizontal: 12,
  },
  bar: {
    flex: 1,
    height: 4,
    borderRadius: 2,
    backgroundColor: "rgba(79, 195, 247, 0.3)",
  },
  low: {
    fontSize: 14,
    color: "#4fc3f7",
    fontWeight: "500",
    fontVariant: ["tabular-nums"],
    width: 35,
    textAlign: "right",
  },
  high: {
    fontSize: 14,
    color: "#ff9800",
    fontWeight: "500",
    fontVariant: ["tabular-nums"],
    width: 35,
  },
  precip: {
    width: 40,
    fontSize: 12,
    color: "#4fc3f7",
    textAlign: "right",
  },
});
```

- [ ] **Step 2: Add DailyForecast to ForecastSheet**

Replace `src/components/forecast/ForecastSheet.tsx`:

```tsx
import { useCallback, useMemo, useRef } from "react";
import { View, Text, StyleSheet } from "react-native";
import { ScrollView } from "react-native-gesture-handler";
import BottomSheet from "@gorhom/bottom-sheet";
import { useForecast } from "../../hooks/useForecast";
import { CurrentConditions } from "./CurrentConditions";
import { HourlyScroll } from "./HourlyScroll";
import { DailyForecast } from "./DailyForecast";

export function ForecastSheet() {
  const bottomSheetRef = useRef<BottomSheet>(null);
  const snapPoints = useMemo(() => [80, "35%", "80%"], []);
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
        <ScrollView>
          <CurrentConditions forecast={forecast} />
          <HourlyScroll forecast={forecast} />
          <DailyForecast forecast={forecast} />
        </ScrollView>
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

- [ ] **Step 3: Commit**

```bash
git add src/components/forecast/DailyForecast.tsx src/components/forecast/ForecastSheet.tsx
git commit -m "feat: add 7-day daily forecast to bottom sheet"
```

---

### Task 8: Settings — Server URL & Data Source

**Files:**
- Modify: `src/app/(tabs)/settings.tsx`

- [ ] **Step 1: Add server URL and data source to settings**

Add after the "Radar" section in `src/app/(tabs)/settings.tsx`, before the footer:

```tsx
      <Section title="Data Source">
        <Row label="Source">
          <SegmentedControl
            options={["Free", "Self-Hosted"]}
            selected={dataSource === "rainviewer" ? "Free" : "Self-Hosted"}
            onSelect={(v) =>
              setDataSource(v === "Free" ? "rainviewer" : "selfhosted")
            }
          />
        </Row>
        {dataSource === "selfhosted" && (
          <Row label="Server URL">
            <TextInput
              style={settingsStyles.textInput}
              value={serverUrl}
              onChangeText={setServerUrl}
              placeholder="http://192.168.1.x:8080"
              placeholderTextColor="#555"
              autoCapitalize="none"
              autoCorrect={false}
            />
          </Row>
        )}
      </Section>
```

Also add these imports and store selectors at the top of the component:

```tsx
import { View, Text, StyleSheet, Switch, TouchableOpacity, TextInput } from "react-native";
// ... existing imports ...

export default function SettingsScreen() {
  // ... existing selectors ...
  const dataSource = useWeatherStore((s) => s.dataSource);
  const setDataSource = useWeatherStore((s) => s.setDataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const setServerUrl = useWeatherStore((s) => s.setServerUrl);
```

Add `textInput` style:

```typescript
  textInput: {
    color: "#fff",
    fontSize: 14,
    backgroundColor: "#333",
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    width: 200,
    textAlign: "right",
  },
```

- [ ] **Step 2: Commit**

```bash
git add src/app/\(tabs\)/settings.tsx
git commit -m "feat: add data source toggle and server URL to settings"
```

---

### Task 9: Map Screen Integration — Layer Picker, Alert Polygon, Overlays

**Files:**
- Modify: `src/app/(tabs)/index.tsx`

- [ ] **Step 1: Update map screen with all new components**

Replace `src/app/(tabs)/index.tsx`:

```tsx
import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { WeatherLayerOverlay } from "../../components/map/WeatherLayerOverlay";
import { AlertPolygon } from "../../components/map/AlertPolygon";
import { TimeSlider } from "../../components/timeline/TimeSlider";
import { PlayButton } from "../../components/timeline/PlayButton";
import { ForecastSheet } from "../../components/forecast/ForecastSheet";
import { AlertBanner } from "../../components/alerts/AlertBanner";
import { LayerPicker } from "../../components/layers/LayerPicker";
import { useLocation } from "../../hooks/useLocation";
import { useManifest } from "../../hooks/useManifest";
import { useWeatherStore } from "../../stores/useWeatherStore";

export default function MapScreen() {
  useLocation();
  useManifest();

  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const visibleOverlays = useWeatherStore((s) => s.visibleOverlays);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const dataSource = useWeatherStore((s) => s.dataSource);

  return (
    <View style={styles.container}>
      <WeatherMap>
        {/* Active fill layer */}
        {activeLayer === "radar" && <RadarOverlay />}
        {activeLayer === "temperature" && dataSource === "selfhosted" && (
          <WeatherLayerOverlay layerId="temperature" opacity={radarOpacity} />
        )}
        {activeLayer === "precip-type" && dataSource === "selfhosted" && (
          <WeatherLayerOverlay layerId="precip-type" opacity={radarOpacity} />
        )}

        {/* Overlay layers (non-exclusive) */}
        {visibleOverlays.has("wind") && dataSource === "selfhosted" && (
          <WeatherLayerOverlay layerId="wind" opacity={0.6} />
        )}
        {visibleOverlays.has("cape") && dataSource === "selfhosted" && (
          <WeatherLayerOverlay layerId="cape" opacity={0.5} />
        )}

        {/* Alert polygons */}
        <AlertPolygon />
      </WeatherMap>

      <AlertBanner />
      <LayerPicker />

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

- [ ] **Step 2: Run tests**

```bash
npx jest --no-cache
```

- [ ] **Step 3: Commit**

```bash
git add src/app/\(tabs\)/index.tsx
git commit -m "feat: integrate layer picker, alert polygons, and multi-layer overlays into map screen"
```

---

### Task 10: Final TypeScript Check & Cleanup

- [ ] **Step 1: Run TypeScript check**

```bash
npx tsc --noEmit
```

Fix any type errors.

- [ ] **Step 2: Run full test suite**

```bash
npx jest --no-cache
```

- [ ] **Step 3: Commit any fixes**

```bash
git add -A
git commit -m "chore: fix type errors and final cleanup for Phase 2-5 app enhancements"
```

---

## Summary of What Phase 2-5 Delivers

**Backend (7 tasks):**
- MRMS radar ingest → 1km resolution tiles every 2 minutes
- HRRR forecast ingest → reflectivity, wind, temperature, CAPE, precip type tiles hourly
- Caddy tile server with manifest API and Open-Meteo proxy
- Docker Compose for local development
- Tile cleanup cron

**App (10 tasks):**
- Data source toggle: RainViewer (free) vs self-hosted
- Server URL configuration persisted in MMKV
- Layer picker with 5 weather layers (radar, wind, temp, precip type, CAPE)
- Alert polygon rendering on map
- 7-day daily forecast in bottom sheet
- Self-hosted manifest support with automatic frame syncing
- Generic weather layer overlay component
