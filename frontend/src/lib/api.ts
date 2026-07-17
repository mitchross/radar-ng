import { API, SELF_HOSTED } from "./constants";
import { trace } from "./telemetry";
import type {
  OpenMeteoResponse,
  RadarNowcastResponse,
  NWSAlertCollection,
  SelfHostedManifest,
  StormPrefetchPlan,
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

/** Location-sampled pySTEPS/MRMS motion nowcast from the self-hosted stack. */
export async function fetchRadarNowcast(
  serverUrl: string,
  lat: number,
  lon: number,
): Promise<RadarNowcastResponse> {
  return trace(
    "api.fetchRadarNowcast",
    async (span) => {
      const res = await fetch(`${serverUrl}${SELF_HOSTED.NOWCAST_PATH}/${lat}/${lon}`);
      span.setAttribute("http.status_code", res.status);
      if (!res.ok) throw new Error(`Nowcast error: ${res.status}`);
      const body = (await res.json()) as RadarNowcastResponse;
      span.setAttribute("radar.nowcast.points", body.points.length);
      span.setAttribute("radar.nowcast.status", body.status);
      return body;
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
        headers: { "User-Agent": "radar-ng/1.1 (self-hosted-weather-radar)" },
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

export async function fetchStormPrefetchPlan(
  serverUrl: string,
  lat: number,
  lon: number,
  palette: string,
  zoom = 6,
): Promise<StormPrefetchPlan> {
  const params = new URLSearchParams({
    lat: String(lat),
    lon: String(lon),
    zoom: String(zoom),
    palette,
  });
  return trace("api.fetchStormPrefetchPlan", async (span) => {
    const res = await fetch(`${serverUrl}/api/storm-prefetch?${params}`);
    span.setAttribute("http.status_code", res.status);
    if (!res.ok) throw new Error(`Storm prefetch error: ${res.status}`);
    const plan = (await res.json()) as StormPrefetchPlan;
    span.setAttribute("radar.storm_prefetch.tiles", plan.tile_urls.length);
    return plan;
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

export interface ServerStatus {
  status: "ok" | "degraded";
  mrms_age_s: number | null;
  mrms_max_age_s?: number;
  nowcast?: { status?: string; reason?: string };
  reasons?: string[];
  tiles_disk?: {
    total_bytes: number;
    used_bytes: number;
    percent: number;
  } | null;
  checked_at?: string;
}

/**
 * Full /api/health body. The endpoint answers 503 WITH a JSON body when the
 * stack is degraded, so parse regardless of HTTP status; null only means
 * unreachable.
 */
export async function fetchServerStatus(serverUrl: string): Promise<ServerStatus | null> {
  return trace("api.fetchServerStatus", async (span) => {
    try {
      const res = await fetch(`${serverUrl}${SELF_HOSTED.HEALTH_PATH}`, { signal: AbortSignal.timeout(5000) });
      span.setAttribute("http.status_code", res.status);
      return (await res.json()) as ServerStatus;
    } catch {
      span.setAttribute("radar.health.timeout", true);
      return null;
    }
  });
}
