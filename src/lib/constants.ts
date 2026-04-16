import type { LayerConfig } from "../types/weather";

export const API = {
  RAINVIEWER_MANIFEST: "https://api.rainviewer.com/public/weather-maps.json",
  OPEN_METEO: "https://api.open-meteo.com/v1/forecast",
  NWS_ALERTS: "https://api.weather.gov/alerts/active",
} as const;

// Public base map tiles (OpenFreeMap). Used when dataSource !== "selfhosted".
export const MAP_STYLES_PUBLIC = {
  light: "https://tiles.openfreemap.org/styles/liberty",
  dark: "https://tiles.openfreemap.org/styles/dark",
} as const;

// Self-hosted base map (Protomaps served through tile-server Caddy). Paths are
// relative to the configured serverUrl.
export const MAP_STYLES_SELFHOSTED = {
  light: "/basemap/styles/positron.json",
  dark: "/basemap/styles/dark-matter.json",
} as const;

// Legacy alias used by settings/tests — resolves to the public set.
export const MAP_STYLES = MAP_STYLES_PUBLIC;

export function resolveMapStyleUrl(
  dataSource: "rainviewer" | "selfhosted",
  serverUrl: string,
  mapStyle: "light" | "dark"
): string {
  if (dataSource === "selfhosted" && serverUrl) {
    return `${serverUrl}${MAP_STYLES_SELFHOSTED[mapStyle]}`;
  }
  return MAP_STYLES_PUBLIC[mapStyle];
}

export const RADAR = {
  TILE_SIZE: 256,
  COLOR_SCHEME: 6,
  SMOOTH: true,
  SNOW: true,
  MIN_ZOOM: 1,
  MAX_ZOOM: 7,
  DEFAULT_OPACITY: 0.8,
} as const;

export const DEFAULTS = {
  LATITUDE: 42.9634,
  LONGITUDE: -85.6681,
  ZOOM: 8,
  PLAYBACK_FPS: 5,
  MANIFEST_REFETCH_MS: 30_000,
  FORECAST_REFETCH_MS: 15 * 60_000,
  ALERTS_REFETCH_MS: 60_000,
} as const;

export const IEM = {
  BASE: "https://mesonet.agron.iastate.edu/cache/tile.py/1.0.0",
  PRODUCT: "nexrad-n0q",
  MAX_MINUTES_AGO: 50,
  STEP_MINUTES: 5,
  MIN_ZOOM: 1,
  MAX_ZOOM: 12,
} as const;

export const SELF_HOSTED = {
  DEFAULT_URL: "http://10.0.2.2:8080",
  MANIFEST_PATH: "/api/manifest.json",
  TILE_PATTERN: "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
  FORECAST_PATH: "/api/forecast",
  HEALTH_PATH: "/api/health",
  METRICS_PATH: "/api/metrics",
  BASEMAP_TILE_PATTERN: "/basemap/tiles/{z}/{x}/{y}.mvt",
} as const;

export const LAYERS: LayerConfig[] = [
  { id: "radar", label: "Radar", icon: "R", isFillLayer: true, defaultVisible: true, minZoom: 1, maxZoom: 12 },
  { id: "wind", label: "Wind", icon: "W", isFillLayer: false, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "temperature", label: "Temp", icon: "T", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "precip-type", label: "Precip", icon: "P", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "cape", label: "CAPE", icon: "S", isFillLayer: false, defaultVisible: false, minZoom: 1, maxZoom: 9 },
];
