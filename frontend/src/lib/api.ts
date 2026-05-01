import { API, SELF_HOSTED } from "./constants";
import { trace } from "./telemetry";
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
  return trace(
    "api.fetchForecast",
    async (span) => {
      const res = await fetch(`${serverUrl}${SELF_HOSTED.FORECAST_PATH}/${lat}/${lon}`);
      span.setAttribute("http.status_code", res.status);
      if (!res.ok) throw new Error(`Forecast error: ${res.status}`);
      return res.json();
    },
    { "geo.lat": lat, "geo.lon": lon },
  );
}

/** NWS active alerts — the one non-self-hosted call (gov API, free, no auth). */
export async function fetchAlerts(
  lat: number,
  lon: number
): Promise<NWSAlertCollection> {
  return trace(
    "api.fetchAlerts",
    async (span) => {
      const res = await fetch(`${API.NWS_ALERTS}?point=${lat},${lon}`, {
        headers: { "User-Agent": "radar-ng/1.0 (self-hosted-weather-radar)" },
      });
      span.setAttribute("http.status_code", res.status);
      if (!res.ok) throw new Error(`NWS API error: ${res.status}`);
      return res.json();
    },
    { "geo.lat": lat, "geo.lon": lon },
  );
}

export async function fetchSelfHostedManifest(
  serverUrl: string
): Promise<SelfHostedManifest> {
  return trace("api.fetchSelfHostedManifest", async (span) => {
    const res = await fetch(`${serverUrl}${SELF_HOSTED.MANIFEST_PATH}`);
    span.setAttribute("http.status_code", res.status);
    if (!res.ok) throw new Error(`Tile server error: ${res.status}`);
    return res.json();
  });
}

export async function checkServerHealth(serverUrl: string): Promise<boolean> {
  return trace("api.checkServerHealth", async (span) => {
    try {
      const res = await fetch(`${serverUrl}${SELF_HOSTED.HEALTH_PATH}`, { signal: AbortSignal.timeout(5000) });
      span.setAttribute("http.status_code", res.status);
      return res.ok;
    } catch {
      span.setAttribute("radar.health.timeout", true);
      return false;
    }
  });
}
