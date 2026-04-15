import { API } from "./constants";
import type {
  RainViewerManifest,
  OpenMeteoResponse,
  NWSAlertCollection,
  SelfHostedManifest,
} from "../types/weather";

export async function fetchRadarManifest(): Promise<RainViewerManifest> {
  const res = await fetch(API.RAINVIEWER_MANIFEST);
  if (!res.ok) throw new Error(`RainViewer API error: ${res.status}`);
  return res.json();
}

export async function fetchForecast(
  lat: number,
  lon: number
): Promise<OpenMeteoResponse> {
  const params = new URLSearchParams({
    latitude: String(lat),
    longitude: String(lon),
    current:
      "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m",
    hourly:
      "temperature_2m,precipitation_probability,weather_code,wind_speed_10m",
    daily:
      "temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,sunrise,sunset",
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
