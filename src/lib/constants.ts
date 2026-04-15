import type { LayerConfig } from "../types/weather";

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

export const SELF_HOSTED = {
  DEFAULT_URL: "http://10.0.2.2:8080",
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
