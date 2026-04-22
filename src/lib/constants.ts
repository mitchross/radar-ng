import type { LayerConfig } from "../types/weather";

export const API = {
  RAINVIEWER_MANIFEST: "https://api.rainviewer.com/public/weather-maps.json",
  OPEN_METEO: "https://api.open-meteo.com/v1/forecast",
  NWS_ALERTS: "https://api.weather.gov/alerts/active",
} as const;

// Public base map tiles (OpenFreeMap + Esri World Imagery for satellite).
export const MAP_STYLES_PUBLIC = {
  light: "https://tiles.openfreemap.org/styles/liberty",
  dark: "https://tiles.openfreemap.org/styles/dark",
  satellite:
    "https://raw.githubusercontent.com/go-spatial/tegola/master/cmd/internal/register/testdata/style.json",
} as const;

// Self-hosted base map styles. The tile-server's /api/basemap/style/{name}
// endpoint rewrites relative tile URLs into absolute ones so MapLibre Native
// can resolve them without a base URL. Satellite falls back to public.
export const MAP_STYLES_SELFHOSTED = {
  light: "/api/basemap/style/positron",
  dark: "/api/basemap/style/dark-matter",
  satellite: MAP_STYLES_PUBLIC.satellite,
} as const;

// Legacy alias used by settings/tests — resolves to the public set.
export const MAP_STYLES = MAP_STYLES_PUBLIC;

export function resolveMapStyleUrl(
  dataSource: "rainviewer" | "selfhosted",
  serverUrl: string,
  mapStyle: "light" | "dark" | "satellite"
): string {
  if (mapStyle === "satellite") return MAP_STYLES_PUBLIC.satellite;
  if (dataSource === "selfhosted" && serverUrl) {
    const path = MAP_STYLES_SELFHOSTED[mapStyle];
    return path.startsWith("http") ? path : `${serverUrl}${path}`;
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
  DEFAULT_URL: "https://radar-ng-api.vanillax.me",
  MANIFEST_PATH: "/api/manifest.json",
  TILE_PATTERN: "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
  FORECAST_PATH: "/api/forecast",
  HEALTH_PATH: "/api/health",
  METRICS_PATH: "/api/metrics",
  BASEMAP_TILE_PATTERN: "/basemap/tiles/{z}/{x}/{y}.mvt",
} as const;

export const LAYERS: LayerConfig[] = [
  { id: "radar", label: "Radar", icon: "R", isFillLayer: true, defaultVisible: true, minZoom: 1, maxZoom: 12 },
  { id: "radar-hrrr", label: "HRRR", icon: "H", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "wind", label: "Wind", icon: "W", isFillLayer: false, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "temperature", label: "Temp", icon: "T", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "precip-type", label: "Precip", icon: "P", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "precip-accum", label: "Rain 1h", icon: "A", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "cloud", label: "Clouds", icon: "C", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "cape", label: "CAPE", icon: "S", isFillLayer: false, defaultVisible: false, minZoom: 1, maxZoom: 9 },
];
