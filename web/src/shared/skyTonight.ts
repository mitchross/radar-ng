/**
 * "Look up tonight": ISS passes, planets, moon and a clear-or-cloudy call for
 * the web panel and the app. Passes come from our server
 * (GET /api/sky/tonight/{lat}/{lon}, SGP4 on CelesTrak elements); planets,
 * moon and twilight come from astro.ts on the device. Pure, so both clients share it.
 */
import { bodyAltitude, bodyRiseSet, clockLabel, moonPhase, sunTimes, twilight, type Planet } from "./astro";
import { forecastTimeMs } from "./forecastTime";
import { ECCC_CREDIT, bestWindowLine, parseStargazing, skyQualityLine, type StargazingTonight } from "./stargazingLayer";

export const SKY_CREDIT = "Orbital elements: CelesTrak";
/** The card shows from this long before sunset until sunrise; the "Tonight" link works any time. */
export const SHOW_BEFORE_SUNSET_MS = 2 * 60 * 60_000;
/** A planet counts as up when at least this high while it is dark. */
export const PLANET_MIN_ALT_DEG = 10;
/** A pass worth a notification: at least this high, with the sky not cloudy at that hour. */
export const GOOD_PASS_MIN_ELEVATION_DEG = 25;
export const ISS_ALERT_LEAD_MS = 10 * 60_000;
export const CLEAR_MAX_PCT = 30;
export const CLOUDY_MIN_PCT = 70;

const HOUR_MS = 60 * 60_000;
const DAY_MS = 24 * HOUR_MS;
const PLANETS: Planet[] = ["Mercury", "Venus", "Mars", "Jupiter", "Saturn"];
const COMPASS = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];

/** Coordinates are rounded so a full fix never leaves the device. */
export function skyTonightPath(lat: number, lon: number): string {
  return `/api/sky/tonight/${lat.toFixed(2)}/${lon.toFixed(2)}`;
}

export interface IssPass {
  start: number;
  maxAt: number;
  end: number;
  maxElevationDeg: number;
  startAzimuthDeg: number;
  endAzimuthDeg: number;
}

export type IssStatus = "ok" | "stale" | "no_data";

export interface IssReport {
  status: IssStatus;
  passes: IssPass[];
  /** Present only when the server publishes the stargazing layer. */
  stargazing?: StargazingTonight | null;
}

/** The server body, or null when it is not one. Malformed passes are dropped. */
export function parseSkyTonight(body: unknown): IssReport | null {
  const b = body as { status?: unknown; passes?: unknown; stargazing?: unknown } | null;
  if (!b || typeof b !== "object") return null;
  const status: IssStatus = b.status === "ok" || b.status === "stale" ? b.status : "no_data";
  const list = Array.isArray(b.passes) ? b.passes : [];
  const passes: IssPass[] = [];
  for (const raw of list) {
    const p = raw as Record<string, unknown>;
    const t = (k: string) => (typeof p[k] === "string" ? Date.parse(p[k] as string) : NaN);
    const n = (k: string) => (typeof p[k] === "number" && Number.isFinite(p[k]) ? (p[k] as number) : NaN);
    const pass = {
      start: t("start"),
      maxAt: t("max_at"),
      end: t("end"),
      maxElevationDeg: n("max_elevation_deg"),
      startAzimuthDeg: n("start_azimuth_deg"),
      endAzimuthDeg: n("end_azimuth_deg"),
    };
    if (Object.values(pass).every(Number.isFinite)) passes.push(pass);
  }
  const stargazing = parseStargazing(b.stargazing);
  return {
    status,
    passes: status === "ok" ? passes.sort((a, b) => a.start - b.start) : [],
    ...(stargazing ? { stargazing } : {}),
  };
}

/** 16-point compass name of an azimuth (0 = north, clockwise). */
export function compassPoint(deg: number): string {
  return COMPASS[Math.round((((deg % 360) + 360) % 360) / 22.5) % 16];
}

function shiftDateKey(dateKey: string, days: number): string {
  const d = new Date(Date.parse(`${dateKey}T12:00:00Z`) + days * DAY_MS);
  return d.toISOString().slice(0, 10);
}

export interface Night {
  sunset: number;
  sunrise: number;
  /** Sun 6° down (end and start of civil twilight): the ISS rule's darkness. */
  darkStart: number;
  darkEnd: number;
}

/**
 * Tonight at the place: after midnight it is the night that started yesterday,
 * otherwise the one that starts at today's sunset. Null in polar day or night.
 */
export function tonight(nowMs: number, dateKey: string, lat: number, lon: number): Night | null {
  const today = sunTimes(dateKey, lat, lon);
  const early = today.rise != null && nowMs < today.rise;
  const eveningKey = early ? shiftDateKey(dateKey, -1) : dateKey;
  const morningKey = early ? dateKey : shiftDateKey(dateKey, 1);
  const sunset = sunTimes(eveningKey, lat, lon).set;
  const sunrise = sunTimes(morningKey, lat, lon).rise;
  const darkStart = twilight(eveningKey, lat, lon, "civil").set;
  const darkEnd = twilight(morningKey, lat, lon, "civil").rise;
  if (sunset == null || sunrise == null || darkStart == null || darkEnd == null) return null;
  if (!(sunset < sunrise && darkStart < darkEnd)) return null;
  return { sunset, sunrise, darkStart, darkEnd };
}

/** From two hours before sunset until sunrise. */
export function inSkyWindow(nowMs: number, night: Night | null): boolean {
  return night != null && nowMs >= night.sunset - SHOW_BEFORE_SUNSET_MS && nowMs < night.sunrise;
}

export interface HourlyClouds {
  time: readonly string[];
  cloud_cover?: readonly (number | null)[];
}

/** Cloud cover (percent) for the forecast hour containing `atMs`, or null when unknown. */
export function cloudAt(hourly: HourlyClouds, atMs: number, utcOffsetSeconds?: number): number | null {
  const cover = hourly.cloud_cover;
  if (!cover) return null;
  for (let i = 0; i < hourly.time.length; i++) {
    const t = forecastTimeMs(hourly.time[i], utcOffsetSeconds);
    if (atMs >= t && atMs < t + HOUR_MS) return cover[i] ?? null;
  }
  return null;
}

export type SkyVerdict = "clear" | "partly" | "cloudy";

/** Mean cloud cover over the dark hours still ahead tonight, as a call. Null without cloud data. */
export function skyVerdict(hourly: HourlyClouds, night: Night, nowMs: number, utcOffsetSeconds?: number): SkyVerdict | null {
  const from = Math.max(night.darkStart, nowMs);
  const values: number[] = [];
  for (let t = Math.floor(from / HOUR_MS) * HOUR_MS; t < night.darkEnd; t += HOUR_MS) {
    const c = cloudAt(hourly, Math.max(t, from), utcOffsetSeconds);
    if (c != null) values.push(c);
  }
  if (!values.length) return null;
  const mean = values.reduce((a, b) => a + b, 0) / values.length;
  return mean <= CLEAR_MAX_PCT ? "clear" : mean >= CLOUDY_MIN_PCT ? "cloudy" : "partly";
}

export const VERDICT_TEXT: Record<SkyVerdict, string> = {
  clear: "Clear, go look",
  partly: "Some clouds, look for gaps",
  cloudy: "Clouds, skip it",
};

export interface PlanetTonight {
  name: Planet;
  /** First and last dark moment it is at least 10° up. */
  from: number;
  until: number;
}

/** Planets at least 10° up at some point while it is dark tonight, sampled every 10 minutes. */
export function planetsTonight(night: Night, lat: number, lon: number): PlanetTonight[] {
  const step = 10 * 60_000;
  const out: PlanetTonight[] = [];
  for (const name of PLANETS) {
    let from: number | null = null;
    let until: number | null = null;
    for (let t = night.darkStart; t <= night.darkEnd; t += step) {
      if (bodyAltitude(name, lat, lon, t) >= PLANET_MIN_ALT_DEG) {
        from ??= t;
        until = t;
      }
    }
    if (from != null && until != null) out.push({ name, from, until });
  }
  return out;
}

export interface SkyClock {
  timeZone?: string;
  utcOffsetSeconds?: number;
}

const clock = (ms: number, c: SkyClock) => clockLabel(ms, c.timeZone, c.utcOffsetSeconds);

export function planetLine(p: PlanetTonight, night: Night, c: SkyClock): string {
  const step = 10 * 60_000;
  const fromDusk = p.from - night.darkStart < step;
  const toDawn = night.darkEnd - p.until < step;
  if (fromDusk && toDawn) return `${p.name} · up all night`;
  if (fromDusk) return `${p.name} · until ${clock(p.until, c)}`;
  if (toDawn) return `${p.name} · from ${clock(p.from, c)}`;
  return `${p.name} · ${clock(p.from, c)} to ${clock(p.until, c)}`;
}

/** "Waxing gibbous · 78% lit · sets 2:14 AM". */
export function moonLine(night: Night, lat: number, lon: number, c: SkyClock): string {
  const phase = moonPhase(night.darkStart);
  const parts = [phase.name, `${Math.round(phase.illuminated * 100)}% lit`];
  const rs = bodyRiseSet("Moon", lat, lon, night.sunset);
  const inNight = (t: number | null): t is number => t != null && t > night.sunset && t < night.sunrise;
  if (inNight(rs.rise)) parts.push(`rises ${clock(rs.rise, c)}`);
  if (inNight(rs.set)) parts.push(`sets ${clock(rs.set, c)}`);
  return parts.join(" · ");
}

export function passLine(p: IssPass, c: SkyClock): string {
  const mins = Math.max(1, Math.round((p.end - p.start) / 60_000));
  return `${clock(p.start, c)} · ${compassPoint(p.startAzimuthDeg)} to ${compassPoint(p.endAzimuthDeg)} · ${Math.round(p.maxElevationDeg)}° high · ${mins} min`;
}

export interface SkyTonightCard {
  verdict: SkyVerdict | null;
  verdictText: string | null;
  /** ISS lines for tonight, empty when none or when hidden (see `issNote`). */
  passes: string[];
  /** Why there are no ISS lines, or the next pass when tonight has none. */
  issNote: string | null;
  planets: string[];
  moon: string;
  /** "Best 10 PM – 2 AM · Excellent" from the stargazing layer, when there is one. */
  bestWindow: string | null;
  /** "Seeing Average · Transparency Good" from ECCC, when the server has it for tonight. */
  skyQuality: string | null;
  /** ECCC credit, set only when `skyQuality` is. */
  seeingCredit: string | null;
  /** The server has the stargazing layer, so the card links to it. */
  starMap: boolean;
  credit: string;
}

/** Passes that start tonight and have not ended. */
export function tonightPasses(report: IssReport | null, night: Night, nowMs: number): IssPass[] {
  return (report?.passes ?? []).filter((p) => p.start >= night.sunset && p.start < night.sunrise && p.end > nowMs);
}

export function skyTonightCard(input: {
  nowMs: number;
  night: Night;
  lat: number;
  lon: number;
  report: IssReport | null;
  hourly: HourlyClouds | null;
  clock: SkyClock;
}): SkyTonightCard {
  const { nowMs, night, lat, lon, report, hourly } = input;
  const c = input.clock;
  const verdict = hourly ? skyVerdict(hourly, night, nowMs, c.utcOffsetSeconds) : null;
  const passes = tonightPasses(report, night, nowMs);
  let issNote: string | null = null;
  if (!report) issNote = "ISS passes are loading or unavailable.";
  else if (report.status === "stale") issNote = "ISS passes hidden: the orbit data is more than 14 days old.";
  else if (report.status === "no_data") issNote = "ISS passes unavailable: the server has no orbit data yet.";
  else if (!passes.length) {
    const next = report.passes.find((p) => p.start >= night.sunrise);
    issNote = next
      ? `No visible ISS pass tonight. Next: ${new Date(next.start).toLocaleDateString("en-US", { weekday: "short", timeZone: c.timeZone })} ${clock(next.start, c)}.`
      : "No visible ISS pass in the next three nights.";
  }
  return {
    verdict,
    verdictText: verdict ? VERDICT_TEXT[verdict] : null,
    passes: passes.map((p) => passLine(p, c)),
    issNote,
    planets: planetsTonight(night, lat, lon).map((p) => planetLine(p, night, c)),
    moon: moonLine(night, lat, lon, c),
    bestWindow: bestWindowLine(report?.stargazing?.best, nowMs, c),
    skyQuality: skyQualityLine(report?.stargazing?.quality),
    seeingCredit: report?.stargazing?.quality ? ECCC_CREDIT : null,
    starMap: report?.stargazing != null,
    credit: SKY_CREDIT,
  };
}

export interface IssAlertPlan {
  /** Stable per pass: the start minute. */
  key: string;
  fireAt: number;
  startAt: number;
  title: string;
  body: string;
}

/**
 * Notifications for good passes still ahead: at least 25° high and, when the
 * forecast covers the hour, cloud cover under the "cloudy" line. Fires 10 minutes before.
 */
export function planIssAlerts(report: IssReport | null, hourly: HourlyClouds | null, nowMs: number, c: SkyClock): IssAlertPlan[] {
  if (!report || report.status !== "ok") return [];
  return report.passes
    .filter((p) => p.maxElevationDeg >= GOOD_PASS_MIN_ELEVATION_DEG && p.start - ISS_ALERT_LEAD_MS > nowMs)
    .filter((p) => {
      const cloud = hourly ? cloudAt(hourly, p.maxAt, c.utcOffsetSeconds) : null;
      return cloud == null || cloud < CLOUDY_MIN_PCT;
    })
    .map((p) => ({
      key: `iss-${Math.floor(p.start / 60_000)}`,
      fireAt: p.start - ISS_ALERT_LEAD_MS,
      startAt: p.start,
      title: "ISS overhead in 10 minutes",
      body: `Look ${compassPoint(p.startAzimuthDeg)} at ${clock(p.start, c)}. It climbs to ${Math.round(p.maxElevationDeg)}° and sets ${compassPoint(p.endAzimuthDeg)}.`,
    }));
}
