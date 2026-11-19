/**
 * Sun, moon and planet times for the web radar and the app, from
 * astronomy-engine (MIT, pure JS, no network). The one place either client
 * touches the library, so the sky cards share it.
 */
import { Body, Equator, Horizon, Illumination, MoonPhase, Observer, SearchAltitude, SearchRiseSet } from "astronomy-engine";

const DAY_MS = 86_400_000;
const DATE_KEY = /^(\d{4})-(\d{2})-(\d{2})$/;

export type Planet = "Mercury" | "Venus" | "Mars" | "Jupiter" | "Saturn";
export type SkyBody = Planet | "Moon";
export type Twilight = "civil" | "nautical" | "astronomical";

const TWILIGHT_DEG: Record<Twilight, number> = { civil: -6, nautical: -12, astronomical: -18 };

export interface RiseSet {
  /** Epoch ms, or null when the body does not cross the horizon in the window (polar day or night). */
  rise: number | null;
  set: number | null;
}

/**
 * Local mean solar midnight that starts the calendar day `dateKey` at
 * `lon`. Searching from here finds that day's events without knowing the
 * time zone, and stays right across clock changes.
 */
export function solarDayStartMs(dateKey: string, lon: number): number {
  const m = DATE_KEY.exec(dateKey);
  if (!m) return NaN;
  return Date.UTC(+m[1], +m[2] - 1, +m[3]) - (lon / 360) * DAY_MS;
}

const ms = (t: { date: Date } | null) => (t ? t.date.getTime() : null);

function riseSetFrom(body: Body, lat: number, lon: number, startMs: number): RiseSet {
  if (!Number.isFinite(startMs)) return { rise: null, set: null };
  const obs = new Observer(lat, lon, 0);
  const start = new Date(startMs);
  return { rise: ms(SearchRiseSet(body, obs, +1, start, 1)), set: ms(SearchRiseSet(body, obs, -1, start, 1)) };
}

/** Sunrise and sunset on the location's calendar day `dateKey` ("2026-12-25"). */
export function sunTimes(dateKey: string, lat: number, lon: number): RiseSet {
  return riseSetFrom(Body.Sun, lat, lon, solarDayStartMs(dateKey, lon));
}

/** Morning twilight start (`rise`) and evening twilight end (`set`) on `dateKey`. */
export function twilight(dateKey: string, lat: number, lon: number, kind: Twilight = "civil"): RiseSet {
  const startMs = solarDayStartMs(dateKey, lon);
  if (!Number.isFinite(startMs)) return { rise: null, set: null };
  const obs = new Observer(lat, lon, 0);
  const start = new Date(startMs);
  const deg = TWILIGHT_DEG[kind];
  return { rise: ms(SearchAltitude(Body.Sun, obs, +1, start, 1, deg)), set: ms(SearchAltitude(Body.Sun, obs, -1, start, 1, deg)) };
}

/** Next rise and next set of a planet or the moon in the 24 h after `fromMs`. */
export function bodyRiseSet(body: SkyBody, lat: number, lon: number, fromMs: number): RiseSet {
  return riseSetFrom(Body[body], lat, lon, fromMs);
}

/** Geometric degrees above the horizon (no refraction, like `twilight`) of a planet, the moon or the sun. */
export function bodyAltitude(body: SkyBody | "Sun", lat: number, lon: number, atMs: number): number {
  const obs = new Observer(lat, lon, 0);
  const at = new Date(atMs);
  const eq = Equator(Body[body], at, obs, true, true);
  return Horizon(at, obs, eq.ra, eq.dec).altitude;
}

export type MoonPhaseName =
  | "New moon"
  | "Waxing crescent"
  | "First quarter"
  | "Waxing gibbous"
  | "Full moon"
  | "Waning gibbous"
  | "Last quarter"
  | "Waning crescent";

const PHASE_NAMES: MoonPhaseName[] = [
  "New moon",
  "Waxing crescent",
  "First quarter",
  "Waxing gibbous",
  "Full moon",
  "Waning gibbous",
  "Last quarter",
  "Waning crescent",
];

/** Moon phase at an instant: angle 0–360 (0 new, 180 full), lit fraction 0–1, and its name. */
export function moonPhase(atMs: number): { angle: number; illuminated: number; name: MoonPhaseName } {
  const at = new Date(atMs);
  const angle = MoonPhase(at);
  const illuminated = Illumination(Body.Moon, at).phase_fraction;
  return { angle, illuminated, name: PHASE_NAMES[Math.floor(((angle + 22.5) % 360) / 45)] };
}

/**
 * "5:05 PM" at the location. Uses the IANA zone when the forecast gave one,
 * else the fixed UTC offset, else the device zone.
 */
export function clockLabel(atMs: number, timeZone?: string, utcOffsetSeconds?: number): string {
  if (timeZone) {
    try {
      return new Intl.DateTimeFormat("en-US", { timeZone, hour: "numeric", minute: "2-digit" }).format(new Date(atMs));
    } catch {
      // Unknown zone name: fall through to the offset.
    }
  }
  const at = utcOffsetSeconds == null ? new Date(atMs) : new Date(atMs + utcOffsetSeconds * 1000);
  const h = utcOffsetSeconds == null ? at.getHours() : at.getUTCHours();
  const min = utcOffsetSeconds == null ? at.getMinutes() : at.getUTCMinutes();
  return `${h % 12 || 12}:${String(min).padStart(2, "0")} ${h < 12 ? "AM" : "PM"}`;
}
