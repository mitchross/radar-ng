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
  /**
   * Optional source layer — only set on merged radar timelines so the
   * overlay knows which subtree to pull from (past=radar, nowcast=nowcast,
   * future=radar-hrrr). Omitted on single-source timelines.
   */
  source?: "radar" | "nowcast" | "radar-hrrr";
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
export type MapStyle = "light" | "dark" | "satellite";
export type MapProjection = "flat" | "globe";
export type Palette = "classic" | "vivid" | "muted";
export type TimelineMode = "current" | "forecast";

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
  | "precip-type"
  | "precip-accum"
  | "cloud";

export type DataSource = "rainviewer" | "selfhosted";

export interface LayerConfig {
  id: LayerType;
  label: string;
  icon: string;
  isFillLayer: boolean;
  defaultVisible: boolean;
  minZoom: number;
  maxZoom: number;
}
