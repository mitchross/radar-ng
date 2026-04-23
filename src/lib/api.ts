import { API, SELF_HOSTED } from "./constants";
import type {
  OpenMeteoResponse,
  NWSAlertCollection,
  SelfHostedManifest,
} from "../types/weather";

/** Forecast — always proxied through the tile-server → open-meteo container. */
export async function fetchForecast(
  serverUrl: string,
  lat: number,
  lon: number,
): Promise<OpenMeteoResponse> {
  const res = await fetch(`${serverUrl}${SELF_HOSTED.FORECAST_PATH}/${lat}/${lon}`);
  if (!res.ok) throw new Error(`Forecast error: ${res.status}`);
  return res.json();
}

/** NWS active alerts — the one non-self-hosted call (gov API, free, no auth). */
export async function fetchAlerts(
  lat: number,
  lon: number
): Promise<NWSAlertCollection> {
  const res = await fetch(`${API.NWS_ALERTS}?point=${lat},${lon}`, {
    headers: { "User-Agent": "radar-ng/1.0 (self-hosted-weather-radar)" },
  });
  if (!res.ok) throw new Error(`NWS API error: ${res.status}`);
  return res.json();
}

export async function fetchSelfHostedManifest(
  serverUrl: string
): Promise<SelfHostedManifest> {
  const res = await fetch(`${serverUrl}${SELF_HOSTED.MANIFEST_PATH}`);
  if (!res.ok) throw new Error(`Tile server error: ${res.status}`);
  return res.json();
}

export async function checkServerHealth(serverUrl: string): Promise<boolean> {
  try {
    const res = await fetch(`${serverUrl}${SELF_HOSTED.HEALTH_PATH}`, { signal: AbortSignal.timeout(5000) });
    return res.ok;
  } catch {
    return false;
  }
}
