import type { LayerConfig } from "../types/weather";

// NWS alerts are the one remaining non-self-hosted dependency (free US-gov
// API, no auth, stable). Everything else goes through the tile-server at
// SELF_HOSTED.DEFAULT_URL.
export const API = {
  NWS_ALERTS: "https://api.weather.gov/alerts/active",
} as const;

// Basemap styles live in the tile-server image at /srv/basemap/styles/.
// Style documents are served by the self-hosted API. The satellite document
// references Esri's public no-key imagery tiles and carries attribution.
export const MAP_STYLES_SELFHOSTED = {
  light: "/basemap/styles/positron.json",
  dark: "/basemap/styles/dark-matter.json",
  satellite: "/basemap/styles/satellite.json",
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
  NOWCAST_PATH: "/api/nowcast",
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
// minZoom: 4 across the board because the cluster's tile pyramids start
// at z=4 (CONUS-only data — there's no meaningful sub-z=4 world tile).
// Setting this to 1 made MapLibre fire 404-bound /1/0/0.png and
// /2/.../.png requests through the public Cloudflare hop, each costing
// ~250 ms RTT and frequently triggering MapLibre's per-source timeout.
export const LAYERS: LayerConfig[] = [
  { id: "radar", label: "Radar", icon: "R", isFillLayer: true, defaultVisible: true, minZoom: 4, maxZoom: 12 },
  { id: "wind", label: "Wind", icon: "W", isFillLayer: false, defaultVisible: false, minZoom: 4, maxZoom: 9 },
  { id: "temperature", label: "Temp", icon: "T", isFillLayer: true, defaultVisible: false, minZoom: 4, maxZoom: 9 },
  { id: "precip-type", label: "Precip", icon: "P", isFillLayer: true, defaultVisible: false, minZoom: 4, maxZoom: 9 },
  { id: "precip-accum", label: "Rain 1h", icon: "A", isFillLayer: true, defaultVisible: false, minZoom: 4, maxZoom: 9 },
  { id: "cloud", label: "Clouds", icon: "C", isFillLayer: true, defaultVisible: false, minZoom: 4, maxZoom: 9 },
  { id: "cape", label: "CAPE", icon: "S", isFillLayer: false, defaultVisible: false, minZoom: 4, maxZoom: 9 },
];
