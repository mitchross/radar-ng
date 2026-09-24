export interface RadarFrame {
  time: number; // Unix epoch seconds
  /** ISO timestamp string used as the tile-server path segment. */
  path: string;
  /** Valid time stays separate because immutable model paths include run_id. */
  timestamp: string;
  /**
   * Optional source layer — only set on merged radar timelines so the
   * overlay knows which subtree to pull from (past=radar, nowcast=nowcast,
   * future=radar-hrrr). Omitted on single-source timelines.
   */
  source?: "radar" | "nowcast" | "radar-hrrr";
  kind?: "observation" | "nowcast" | "model_guidance";
  issuedAt?: string;
  leadMinutes?: number;
  spatialResolutionKm?: number;
  maxZoom?: number;
}

// --- Open-Meteo API ---

/**
 * Open-Meteo returns `null` for any value a model doesn't provide (e.g. hours
 * past a model's horizon). Types say so, so the UI must render "—" instead of
 * inventing 0° or clear skies.
 */
type Maybe = number | null;

export interface OpenMeteoResponse {
  latitude: number;
  longitude: number;
  current: {
    time: string;
    temperature_2m: Maybe;
    relative_humidity_2m: Maybe;
    apparent_temperature: Maybe;
    weather_code: Maybe;
    wind_speed_10m: Maybe;
    wind_direction_10m: Maybe;
    wind_gusts_10m: Maybe;
    dew_point_2m: Maybe;
    surface_pressure: Maybe;
  };
  hourly: {
    time: string[];
    temperature_2m: Maybe[];
    precipitation_probability: Maybe[];
    precipitation: Maybe[];
    weather_code: Maybe[];
    wind_speed_10m: Maybe[];
    visibility?: Maybe[];
  };
  daily: {
    time: string[];
    temperature_2m_max: Maybe[];
    temperature_2m_min: Maybe[];
    weather_code: Maybe[];
    precipitation_sum: Maybe[];
    precipitation_probability_max: Maybe[];
    uv_index_max: Maybe[];
    sunrise: (string | null)[];
    sunset: (string | null)[];
  };
  minutely_15?: {
    time: string[];
    precipitation: Maybe[];
    precipitation_probability?: Maybe[];
  };
}

export interface RadarNowcastPoint {
  timestamp: string;
  lead_minutes: number | null;
  dbz: number | null;
  precipitation_mm_h: number;
}

export interface RadarNowcastResponse {
  status: "ok" | "degraded" | "unavailable";
  source?: "mrms-nowcast";
  method?: string;
  issued_at?: string;
  horizon_minutes?: number;
  step_minutes?: number;
  spatial_resolution_km?: number;
  latitude?: number;
  longitude?: number;
  reason?: string;
  detail?: string | null;
  points: RadarNowcastPoint[];
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
    instruction: string | null;
    severity: "Extreme" | "Severe" | "Moderate" | "Minor" | "Unknown";
    urgency: "Immediate" | "Expected" | "Future" | "Past" | "Unknown";
    onset: string | null;
    effective: string;
    expires: string;
    ends: string | null;
    areaDesc: string;
    senderName: string | null;
  };
}

// --- App State ---

export type TemperatureUnit = "fahrenheit" | "celsius";
export type WindUnit = "mph" | "kmh";
export type MapStyle = "light" | "dark" | "satellite";
export type MapProjection = "flat" | "globe";
export type Palette = "classic" | "vivid" | "muted";
export type TimelineMode = "current" | "forecast";

// --- Self-Hosted Tile Server ---

export interface SelfHostedManifest {
  schema_version?: number;
  layers: Record<string, {
    timestamps: string[];
    frames?: {
      timestamp: string;
      path: string;
      source?: string;
      kind?: "observation" | "nowcast" | "model_guidance";
      issued_at?: string;
      lead_minutes?: number;
      spatial_resolution_km?: number;
      max_zoom?: number;
      palettes?: string[];
    }[];
    latest?: string;
    title?: string;
    kind?: string;
    complete?: boolean;
  }>;
  tile_url_template: string;
  updated_at: string;
}

export interface StormPrefetchBBox {
  lead_minutes: 0 | 5 | 10;
  bbox: [number, number, number, number];
  layer: "radar" | "nowcast" | null;
  timestamp: string | null;
  zoom: number;
  style_url: string | null;
  tile_urls: string[];
}

export interface StormPrefetchPlan {
  plan_id: string | null;
  storm_cell_id: number | null;
  generated_at?: number;
  tracking_vector?: {
    east_kmh: number;
    north_kmh: number;
    speed_kmh: number;
    bearing_deg: number;
  };
  bboxes: StormPrefetchBBox[];
  tile_urls: string[];
}

// --- Layers ---

export type LayerType =
  | "radar"
  | "radar-composite"
  | "radar-hrrr"
  | "temperature"
  | "wind"
  | "cape"
  | "precip-type"
  | "precip-accum"
  | "cloud"
  | "air-quality"
  | "ozone";

export interface LayerConfig {
  id: LayerType;
  label: string;
  icon: string;
  isFillLayer: boolean;
  defaultVisible: boolean;
  minZoom: number;
  maxZoom: number;
}
