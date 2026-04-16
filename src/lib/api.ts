import { API, SELF_HOSTED } from "./constants";
import type {
  RadarFrame,
  RainViewerManifest,
  OpenMeteoResponse,
  NWSAlertCollection,
  SelfHostedManifest,
  DataSource,
} from "../types/weather";

export async function fetchRadarManifest(): Promise<RainViewerManifest> {
  const res = await fetch(API.RAINVIEWER_MANIFEST);
  if (!res.ok) throw new Error(`RainViewer API error: ${res.status}`);
  return res.json();
}

interface ForecastOptions {
  dataSource?: DataSource;
  serverUrl?: string;
}

export async function fetchForecast(
  lat: number,
  lon: number,
  opts: ForecastOptions = {}
): Promise<OpenMeteoResponse> {
  // Self-hosted: route through the tile-server proxy. The server talks to the
  // local Open-Meteo container when OPEN_METEO_BASE points at it.
  if (opts.dataSource === "selfhosted" && opts.serverUrl) {
    const url = `${opts.serverUrl}${SELF_HOSTED.FORECAST_PATH}/${lat}/${lon}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`Self-hosted forecast error: ${res.status}`);
    return res.json();
  }

  // Public path — straight to api.open-meteo.com.
  const params = new URLSearchParams({
    latitude: String(lat),
    longitude: String(lon),
    current:
      "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m,dew_point_2m,surface_pressure",
    hourly:
      "temperature_2m,precipitation_probability,weather_code,wind_speed_10m,relative_humidity_2m,apparent_temperature",
    daily:
      "temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,precipitation_probability_max,uv_index_max,sunrise,sunset",
    temperature_unit: "fahrenheit",
    wind_speed_unit: "mph",
    precipitation_unit: "inch",
    timezone: "auto",
    forecast_days: "7",
  });
  const res = await fetch(`${API.OPEN_METEO}?${params}`);
  if (!res.ok) throw new Error(`Open-Meteo API error: ${res.status}`);
  return res.json();
}

export async function fetchAlerts(
  lat: number,
  lon: number
): Promise<NWSAlertCollection> {
  const res = await fetch(`${API.NWS_ALERTS}?point=${lat},${lon}`, {
    headers: { "User-Agent": "StormScope/1.0 (weather-radar-app)" },
  });
  if (!res.ok) throw new Error(`NWS API error: ${res.status}`);
  return res.json();
}

export function buildIEMFrames(): RadarFrame[] {
  const now = Math.floor(Date.now() / 1000);
  const frames: RadarFrame[] = [];
  for (let m = 50; m >= 5; m -= 5) {
    const padded = String(m).padStart(2, "0");
    frames.push({
      time: now - m * 60,
      path: `nexrad-n0q-m${padded}m`,
    });
  }
  frames.push({ time: now, path: "nexrad-n0q" });
  return frames;
}

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
