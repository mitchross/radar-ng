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
  COLOR_SCHEME: 1,
  SMOOTH: true,
  SNOW: true,
  MIN_ZOOM: 1,
  MAX_ZOOM: 7,
  DEFAULT_OPACITY: 0.7,
} as const;

export const DEFAULTS = {
  LATITUDE: 39.8283,
  LONGITUDE: -98.5795,
  ZOOM: 4,
  PLAYBACK_FPS: 5,
  MANIFEST_REFETCH_MS: 30_000,
  FORECAST_REFETCH_MS: 15 * 60_000,
  ALERTS_REFETCH_MS: 60_000,
} as const;
