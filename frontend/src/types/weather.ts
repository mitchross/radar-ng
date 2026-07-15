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
    dew_point_2m: number;
    surface_pressure: number;
  };
  hourly: {
    time: string[];
    temperature_2m: number[];
    precipitation_probability: number[];
    precipitation: number[];
    weather_code: number[];
    wind_speed_10m: number[];
    visibility?: number[];
  };
  daily: {
    time: string[];
    temperature_2m_max: number[];
    temperature_2m_min: number[];
    weather_code: number[];
    precipitation_sum: number[];
    precipitation_probability_max: number[];
    uv_index_max: number[];
    sunrise: string[];
    sunset: string[];
  };
  minutely_15?: {
    time: string[];
    precipitation: number[];
    precipitation_probability?: number[];
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
  | "cloud";

export interface LayerConfig {
  id: LayerType;
  label: string;
  icon: string;
  isFillLayer: boolean;
  defaultVisible: boolean;
  minZoom: number;
  maxZoom: number;
}
