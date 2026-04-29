import type { LayerConfig } from "../types/weather";

// NWS alerts are the one remaining non-self-hosted dependency (free US-gov
// API, no auth, stable). Everything else goes through the tile-server at
// SELF_HOSTED.DEFAULT_URL.
export const API = {
  NWS_ALERTS: "https://api.weather.gov/alerts/active",
} as const;

// Basemap styles live in the tile-server image at /srv/basemap/styles/.
// Satellite falls back to a public Esri-style JSON — the others are served
// locally via Caddy.
export const MAP_STYLES_SELFHOSTED = {
  light: "/basemap/styles/positron.json",
  dark: "/basemap/styles/dark-matter.json",
  satellite:
    "https://raw.githubusercontent.com/go-spatial/tegola/master/cmd/internal/register/testdata/style.json",
} as const;

export function resolveMapStyleUrl(
  serverUrl: string,
  mapStyle: "light" | "dark" | "satellite"
): string {
  const path = MAP_STYLES_SELFHOSTED[mapStyle];
  return path.startsWith("http") ? path : `${serverUrl}${path}`;
}

export const RADAR = {
  TILE_SIZE: 256,
  MIN_ZOOM: 1,
  MAX_ZOOM: 12,
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
  DEFAULT_URL: "https://radar-ng-api.vanillax.me",
  MANIFEST_PATH: "/api/manifest.json",
  TILE_PATTERN: "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png",
  FORECAST_PATH: "/api/forecast",
  HEALTH_PATH: "/api/health",
  METRICS_PATH: "/api/metrics",
  BASEMAP_TILE_PATTERN: "/basemap/tiles/{z}/{x}/{y}.mvt",
} as const;

// `radar-composite` / `radar-hrrr` are NOT in this list because the
// tile-server manifest doesn't currently expose them as standalone layers.
// Selecting them previously yielded silent blank overlays + 404s. They
// can come back once the cluster manifest publishes them; until then the
// `radar` layer's forecast mode already merges past + nowcast + HRRR into
// a single timeline.
export const LAYERS: LayerConfig[] = [
  { id: "radar", label: "Radar", icon: "R", isFillLayer: true, defaultVisible: true, minZoom: 1, maxZoom: 12 },
  { id: "wind", label: "Wind", icon: "W", isFillLayer: false, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "temperature", label: "Temp", icon: "T", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "precip-type", label: "Precip", icon: "P", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "precip-accum", label: "Rain 1h", icon: "A", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "cloud", label: "Clouds", icon: "C", isFillLayer: true, defaultVisible: false, minZoom: 1, maxZoom: 9 },
  { id: "cape", label: "CAPE", icon: "S", isFillLayer: false, defaultVisible: false, minZoom: 1, maxZoom: 9 },
];
