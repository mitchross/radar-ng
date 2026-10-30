/**
 * Forecast, place search, alerts and point readings from the self-hosted API.
 * Every request is same-origin (see vite.config.ts); nothing leaves the stack.
 */

import { getCumulusCondition, type CumulusCondition } from "./shared/conditions";

export interface Forecast {
  /** Seconds east of UTC at the location; `hourly.time` etc. are wall times there with no offset. */
  utc_offset_seconds?: number;
  current: {
    temperature_2m: number | null;
    apparent_temperature: number | null;
    relative_humidity_2m: number | null;
    dew_point_2m: number | null;
    surface_pressure: number | null;
    weather_code: number | null;
    wind_speed_10m: number | null;
    wind_gusts_10m: number | null;
    wind_direction_10m: number | null;
  };
  hourly: { time: string[]; temperature_2m: (number | null)[]; precipitation: (number | null)[]; weather_code: (number | null)[] };
  daily: {
    time: string[];
    temperature_2m_max: (number | null)[];
    temperature_2m_min: (number | null)[];
    weather_code: (number | null)[];
    precipitation_sum: (number | null)[];
    uv_index_max?: (number | null)[];
    sunrise: string[];
    sunset: string[];
  };
}

export interface Place {
  name: string;
  latitude: number;
  longitude: number;
  admin1?: string;
}

export interface Alert {
  event: string;
  headline: string;
  severity: string;
  ends: string | null;
}

async function json<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, { signal });
  if (!res.ok) throw new Error(`${url}: ${res.status}`);
  return (await res.json()) as T;
}

const round = (v: number, dp: number) => Number(v.toFixed(dp));

export const fetchManifest = (signal?: AbortSignal) => json<import("./frames").Manifest>("/api/manifest.json", signal);

const WALL_TIME = /^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2}))?$/;

/** Open-Meteo wall time at the location → epoch ms. Falls back to the browser zone without an offset. */
export function forecastTimeMs(value: string, utcOffsetSeconds?: number): number {
  const m = utcOffsetSeconds == null ? null : WALL_TIME.exec(value);
  if (!m || utcOffsetSeconds == null) return Date.parse(value);
  return Date.UTC(+m[1], +m[2] - 1, +m[3], +(m[4] ?? 0), +(m[5] ?? 0)) - utcOffsetSeconds * 1000;
}

/** "3pm" in the location's zone, not the browser's. */
export function forecastHourLabel(value: string, utcOffsetSeconds?: number): string {
  if (utcOffsetSeconds == null) return new Date(value).toLocaleTimeString([], { hour: "numeric" }).replace(" ", "").toLowerCase();
  const h = new Date(forecastTimeMs(value, utcOffsetSeconds) + utcOffsetSeconds * 1000).getUTCHours();
  if (h === 0) return "12am";
  if (h === 12) return "12pm";
  return h < 12 ? `${h}am` : `${h - 12}pm`;
}

/** Night at the location: outside that day's sunrise–sunset, else 20:00–06:00. Mirrors the app's isNightAt. */
export function isNightAt(atMs: number, daily: Pick<Forecast["daily"], "time" | "sunrise" | "sunset">, utcOffsetSeconds?: number): boolean {
  const offset = utcOffsetSeconds ?? -new Date(atMs).getTimezoneOffset() * 60;
  const wall = new Date(atMs + offset * 1000);
  const i = daily.time.indexOf(wall.toISOString().slice(0, 10));
  const rise = i >= 0 && daily.sunrise[i] ? forecastTimeMs(daily.sunrise[i], utcOffsetSeconds) : NaN;
  const set = i >= 0 && daily.sunset[i] ? forecastTimeMs(daily.sunset[i], utcOffsetSeconds) : NaN;
  if (Number.isFinite(rise) && Number.isFinite(set)) return atMs < rise || atMs > set;
  const hour = wall.getUTCHours();
  return hour < 6 || hour >= 20;
}

export const fetchForecast = (lat: number, lon: number, signal?: AbortSignal) =>
  json<Forecast>(`/api/forecast/${round(lat, 2)}/${round(lon, 2)}`, signal);

export async function searchPlaces(q: string, signal?: AbortSignal): Promise<Place[]> {
  const body = await json<{ results: Place[] }>(`/api/geocode?q=${encodeURIComponent(q)}&limit=6`, signal);
  return body.results;
}

export async function reversePlace(lat: number, lon: number): Promise<Place | null> {
  const body = await json<{ place: Place | null }>(`/api/reverse-geocode?lat=${round(lat, 2)}&lon=${round(lon, 2)}`);
  return body.place;
}

export async function fetchAlerts(lat: number, lon: number): Promise<Alert[]> {
  const body = await json<{ features?: { properties: Alert }[] }>(`/api/alerts?lat=${round(lat, 2)}&lon=${round(lon, 2)}`);
  return (body.features ?? []).map((f) => f.properties);
}

export interface Briefing {
  headline: string;
  body: string;
  model?: string;
}

/** Optional LLM summary for a place; null whenever the server has no model or it's down. */
export async function fetchBriefing(place: Place, signal?: AbortSignal): Promise<Briefing | null> {
  const q = new URLSearchParams({ lat: String(round(place.latitude, 2)), lon: String(round(place.longitude, 2)), place: place.name });
  try {
    const body = await json<{ available: boolean } & Partial<Briefing>>(`/api/briefing?${q}`, signal);
    return body.available && body.body ? { headline: body.headline ?? "", body: body.body, model: body.model } : null;
  } catch {
    return null;
  }
}

export async function explainAlert(id: string): Promise<{ what: string; do: string } | null> {
  try {
    const body = await json<{ available: boolean; what?: string; do?: string }>(`/api/alerts/explain?id=${encodeURIComponent(id)}`);
    return body.available && body.what ? { what: body.what, do: body.do ?? "" } : null;
  } catch {
    return null;
  }
}

/** The value of a timeline frame at a point. Nowcast grids are per run, so they come from /api/nowcast's series. */
export async function readPoint(
  layer: string,
  source: string,
  timestamp: string,
  lat: number,
  lon: number,
): Promise<number | null> {
  const la = round(lat, 3);
  const lo = round(lon, 3);
  if (layer === "radar" && source === "nowcast") {
    const body = await json<{ points: { timestamp: string; dbz: number | null }[] }>(`/api/nowcast/${la}/${lo}`);
    const at = Date.parse(timestamp);
    return body.points.find((p) => Date.parse(p.timestamp) === at)?.dbz ?? null;
  }
  const inspectLayer = layer === "radar" ? source : layer;
  const body = await json<{ ok: boolean; value: number | null }>(
    `/api/inspect/${inspectLayer}/${encodeURIComponent(timestamp)}/${la}/${lo}`,
  );
  return body.ok ? body.value : null;
}

export function describeDbz(dbz: number): string {
  if (dbz < 5) return "No precipitation";
  if (dbz < 15) return "Drizzle";
  if (dbz < 25) return "Light";
  if (dbz < 35) return "Moderate";
  if (dbz < 45) return "Heavy";
  if (dbz < 55) return "Intense";
  return "Hail possible";
}

/** EPA PM2.5 breakpoints (2024) → US AQI. */
export function usAqiFromPm25(ugm3: number): number {
  const bp: [number, number, number, number][] = [
    [0, 9.0, 0, 50], [9.1, 35.4, 51, 100], [35.5, 55.4, 101, 150],
    [55.5, 125.4, 151, 200], [125.5, 225.4, 201, 300], [225.5, 325.4, 301, 500],
  ];
  const c = Math.max(0, Math.floor(ugm3 * 10) / 10);
  for (const [cl, ch, il, ih] of bp) if (c <= ch) return Math.round(((ih - il) / (ch - cl)) * (Math.max(c, cl) - cl) + il);
  return 500;
}

export function aqiLabel(aqi: number): string {
  if (aqi <= 50) return "Good";
  if (aqi <= 100) return "Moderate";
  if (aqi <= 150) return "Unhealthy for sensitive groups";
  if (aqi <= 200) return "Unhealthy";
  if (aqi <= 300) return "Very unhealthy";
  return "Hazardous";
}

export type Condition = "clear" | "partly" | "cloudy" | "fog" | "rain" | "snow" | "storm";

const ICON_OF_SKY: Record<CumulusCondition, Condition> = {
  clearDay: "clear", clearNight: "clear", cloudy: "cloudy", rain: "rain",
  storm: "storm", snow: "snow", fog: "fog", unknown: "cloudy",
};

/**
 * Icon for a WMO code, from the same mapping as the headline (getCumulusCondition).
 * Like the app's getIconKind, codes 1 and 2 are partly cloudy (the web has no moon icon).
 */
export function conditionOf(code: number | null): Condition {
  return code === 1 || code === 2 ? "partly" : ICON_OF_SKY[getCumulusCondition(code, false)];
}

const CLOUD = '<path d="M7 18h10a4 4 0 0 0 .6-7.96A6 6 0 0 0 6.2 9.1 4.5 4.5 0 0 0 7 18z" fill="#aab3c8"/>';
const SUN = (cx: number, cy: number, r: number) =>
  `<circle cx="${cx}" cy="${cy}" r="${r}" fill="#f2b233"/>`;

/** Small inline SVG weather icons (no icon font or CDN). */
export function iconSvg(c: Condition, size = 28): string {
  const body: Record<Condition, string> = {
    clear: SUN(12, 12, 5.5),
    partly: `${SUN(9, 9, 4.5)}${CLOUD}`,
    cloudy: CLOUD,
    fog: '<g stroke="#9aa4be" stroke-width="2" stroke-linecap="round"><path d="M4 8h16M6 12h14M4 16h16M8 20h10"/></g>',
    rain: `${CLOUD}<g stroke="#4d7fb8" stroke-width="1.8" stroke-linecap="round"><path d="M9 20l-1 2.5M13 20l-1 2.5M17 20l-1 2.5"/></g>`,
    snow: `${CLOUD}<g fill="#7fa7e8"><circle cx="9" cy="21" r="1.2"/><circle cx="13" cy="22" r="1.2"/><circle cx="17" cy="21" r="1.2"/></g>`,
    storm: `${CLOUD}<path d="M12.5 17l-2.5 4h2l-1 3.5 3.5-5h-2l1.2-2.5z" fill="#f2b233"/>`,
  };
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true">${body[c]}</svg>`;
}
