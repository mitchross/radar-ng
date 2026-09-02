import { API, SELF_HOSTED } from "./constants";
import { trace } from "./telemetry";
import { parseSelfHostedManifest } from "./manifest";
import type {
  OpenMeteoResponse,
  RadarNowcastResponse,
  NWSAlertCollection,
  SelfHostedManifest,
  StormPrefetchPlan,
} from "../types/weather";

export const FETCH_TIMEOUT_MS = 10_000;

/**
 * fetch() with a hard deadline, chained to an optional caller signal (react-query's).
 * Hermes has no AbortSignal.timeout/any (RN polyfills abort-controller@3), so wire it by hand.
 */
export async function fetchWithTimeout(
  url: string,
  init: RequestInit = {},
  signal?: AbortSignal,
  timeoutMs = FETCH_TIMEOUT_MS,
): Promise<Response> {
  const ctrl = new AbortController();
  const onAbort = () => ctrl.abort();
  if (signal?.aborted) ctrl.abort();
  else signal?.addEventListener("abort", onAbort);
  const timer = setTimeout(onAbort, timeoutMs);
  try {
    return await fetch(url, { ...init, signal: ctrl.signal });
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener("abort", onAbort);
  }
}

/** Forecast — always proxied through the tile-server → open-meteo container. */
export async function fetchForecast(
  serverUrl: string,
  lat: number,
  lon: number,
  signal?: AbortSignal,
): Promise<OpenMeteoResponse> {
  return trace(
    "api.fetchForecast",
    async (span) => {
      const res = await fetchWithTimeout(`${serverUrl}${SELF_HOSTED.FORECAST_PATH}/${lat}/${lon}`, {}, signal);
      span.setAttribute("http.status_code", res.status);
      if (!res.ok) throw new Error(`Forecast error: ${res.status}`);
      return res.json();
    },
  );
}

/** Location-sampled pySTEPS/MRMS motion nowcast from the self-hosted stack. */
export async function fetchRadarNowcast(
  serverUrl: string,
  lat: number,
  lon: number,
  signal?: AbortSignal,
): Promise<RadarNowcastResponse> {
  return trace(
    "api.fetchRadarNowcast",
    async (span) => {
      const res = await fetchWithTimeout(`${serverUrl}${SELF_HOSTED.NOWCAST_PATH}/${lat}/${lon}`, {}, signal);
      span.setAttribute("http.status_code", res.status);
      if (!res.ok) throw new Error(`Nowcast error: ${res.status}`);
      const body = (await res.json()) as RadarNowcastResponse;
      span.setAttribute("radar.nowcast.points", body.points.length);
      span.setAttribute("radar.nowcast.status", body.status);
      return body;
    },
  );
}

/** NWS active alerts — the one non-self-hosted call (gov API, free, no auth). */
export async function fetchAlerts(
  lat: number,
  lon: number,
  signal?: AbortSignal,
): Promise<NWSAlertCollection> {
  return trace(
    "api.fetchAlerts",
    async (span) => {
      const res = await fetchWithTimeout(
        `${API.NWS_ALERTS}?point=${lat},${lon}`,
        { headers: { "User-Agent": "radar-ng/1.1 (self-hosted-weather-radar)" } },
        signal,
      );
      span.setAttribute("http.status_code", res.status);
      if (!res.ok) throw new Error(`NWS API error: ${res.status}`);
      return res.json();
    },
  );
}

export async function fetchSelfHostedManifest(
  serverUrl: string,
  signal?: AbortSignal,
): Promise<SelfHostedManifest> {
  return trace("api.fetchSelfHostedManifest", async (span) => {
    const res = await fetchWithTimeout(`${serverUrl}${SELF_HOSTED.MANIFEST_PATH}`, {}, signal);
    span.setAttribute("http.status_code", res.status);
    if (!res.ok) throw new Error(`Tile server error: ${res.status}`);
    return parseSelfHostedManifest(await res.json());
  });
}

export async function fetchStormPrefetchPlan(
  serverUrl: string,
  lat: number,
  lon: number,
  palette: string,
  zoom = 6,
  signal?: AbortSignal,
): Promise<StormPrefetchPlan> {
  const params = new URLSearchParams({
    lat: String(lat),
    lon: String(lon),
    zoom: String(zoom),
    palette,
  });
  return trace("api.fetchStormPrefetchPlan", async (span) => {
    const res = await fetchWithTimeout(`${serverUrl}/api/storm-prefetch?${params}`, {}, signal);
    span.setAttribute("http.status_code", res.status);
    if (!res.ok) throw new Error(`Storm prefetch error: ${res.status}`);
    const plan = (await res.json()) as StormPrefetchPlan;
    span.setAttribute("radar.storm_prefetch.tiles", plan.tile_urls.length);
    return plan;
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

export type HealthLevel = "ok" | "degraded" | "error";

/** /api/health answers 503 with a JSON body when data is merely stale — only "no body" means down. */
export function healthLevelOf(status: ServerStatus | null | undefined): HealthLevel {
  if (!status) return "error";
  return status.status === "ok" ? "ok" : "degraded";
}

/** Full /api/health body regardless of HTTP status; null only when unreachable or not JSON. */
export async function fetchServerStatus(
  serverUrl: string,
  signal?: AbortSignal,
): Promise<ServerStatus | null> {
  return trace("api.fetchServerStatus", async (span) => {
    try {
      const res = await fetchWithTimeout(`${serverUrl}${SELF_HOSTED.HEALTH_PATH}`, {}, signal, 5000);
      span.setAttribute("http.status_code", res.status);
      const body = (await res.json()) as Partial<ServerStatus> | null;
      return body && typeof body.status === "string" ? (body as ServerStatus) : null;
    } catch {
      span.setAttribute("radar.health.timeout", true);
      return null;
    }
  });
}
