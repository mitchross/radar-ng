/**
 * Pure derivations for the forecast screens. Every value can be missing in the
 * source (Open-Meteo returns null past a model's horizon), so each result
 * carries null through instead of inventing 0°, 0%, clear skies or a
 * "normal" pressure.
 */
import { getIconKind, type IconKind } from "./cumulusTheme";
import { describeNowcastSkill } from "./nowcastSkill";
import { interpolateRadarNowcast } from "./radarNowcast";
import { getNowcastVerdict } from "./weatherPresentation";
import { displayTemperature } from "./temperature";
import type { OpenMeteoResponse, RadarNowcastResponse, TemperatureUnit } from "../types/weather";

type Daily = OpenMeteoResponse["daily"];
type Minutely = NonNullable<OpenMeteoResponse["minutely_15"]>;

const WALL_TIME = /^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2}))?$/;

/**
 * Open-Meteo returns wall time at the forecast location with no offset
 * ("2026-10-11T14:00"). `new Date()` would read it in the device zone, which
 * shifts a chosen city in another zone by the zone difference. Without an
 * offset (old cached responses), fall back to the device zone as before.
 */
export function parseLocationTime(value: string | null | undefined, utcOffsetSeconds?: number): Date | null {
  if (!value) return null;
  const m = utcOffsetSeconds == null ? null : WALL_TIME.exec(value);
  const d = m
    ? new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +(m[4] ?? 0), +(m[5] ?? 0)) - (utcOffsetSeconds as number) * 1000)
    : new Date(value);
  return Number.isFinite(d.getTime()) ? d : null;
}

/** The instant shifted so its UTC fields read as wall time at the location. */
function wallClock(at: Date, utcOffsetSeconds?: number): { hour: number; dateKey: string } {
  if (utcOffsetSeconds == null) {
    return {
      hour: at.getHours(),
      dateKey: `${at.getFullYear()}-${String(at.getMonth() + 1).padStart(2, "0")}-${String(at.getDate()).padStart(2, "0")}`,
    };
  }
  const w = new Date(at.getTime() + utcOffsetSeconds * 1000);
  return {
    hour: w.getUTCHours(),
    dateKey: `${w.getUTCFullYear()}-${String(w.getUTCMonth() + 1).padStart(2, "0")}-${String(w.getUTCDate()).padStart(2, "0")}`,
  };
}

/** Sunrise/sunset for the location's calendar day that contains `at`, or nulls. */
export function sunTimesFor(daily: Daily, at: Date, utcOffsetSeconds?: number): { sunrise: Date | null; sunset: Date | null } {
  const index = daily.time.indexOf(wallClock(at, utcOffsetSeconds).dateKey);
  if (index < 0) return { sunrise: null, sunset: null };
  return {
    sunrise: parseLocationTime(daily.sunrise[index], utcOffsetSeconds),
    sunset: parseLocationTime(daily.sunset[index], utcOffsetSeconds),
  };
}

/**
 * Night for the hour's own day, not today's. Tomorrow 8 AM compared against
 * today's sunset used to read as night. Without sun times, fall back to a
 * fixed 06:00–20:00 day rather than guessing from today.
 */
export function isNightAt(at: Date, daily: Daily, utcOffsetSeconds?: number): boolean {
  const { sunrise, sunset } = sunTimesFor(daily, at, utcOffsetSeconds);
  if (sunrise && sunset) return at < sunrise || at > sunset;
  const { hour } = wallClock(at, utcOffsetSeconds);
  return hour < 6 || hour >= 20;
}

/** Index of the first hourly entry at or after "now" (with a 30-minute grace). */
export function startHourIndex(hours: string[], now = Date.now(), utcOffsetSeconds?: number): number {
  for (let i = 0; i < hours.length; i++) {
    const t = parseLocationTime(hours[i], utcOffsetSeconds);
    if (t && t.getTime() >= now - 30 * 60_000) return i;
  }
  return 0;
}

function formatHour(d: Date, i: number, utcOffsetSeconds?: number): string {
  if (i === 0) return "NOW";
  const h = wallClock(d, utcOffsetSeconds).hour;
  if (h === 0) return "12a";
  if (h === 12) return "12p";
  return h < 12 ? `${h}a` : `${h - 12}p`;
}

export interface HourView {
  label: string;
  temp: number | null;
  icon: IconKind;
  night: boolean;
  /** Chance of precipitation, 0–100, or null when unknown. */
  chance: number | null;
  isNow: boolean;
}

export function hourlyView(
  forecast: OpenMeteoResponse,
  unit: TemperatureUnit,
  now = Date.now(),
  count = 24,
): HourView[] {
  const { hourly, daily, utc_offset_seconds: offset } = forecast;
  const start = startHourIndex(hourly.time, now, offset);
  return hourly.time.slice(start, start + count).map((t, i) => {
    const idx = start + i;
    const at = parseLocationTime(t, offset) ?? new Date(now);
    const night = isNightAt(at, daily, offset);
    return {
      label: formatHour(at, i, offset),
      temp: displayTemperature(hourly.temperature_2m[idx], unit),
      icon: getIconKind(hourly.weather_code[idx], night),
      night,
      chance: hourly.precipitation_probability?.[idx] ?? null,
      isNow: i === 0,
    };
  });
}

/**
 * Total precipitation over the same 24 hours the hourly strip shows. The card
 * used to show the calendar day's `precipitation_sum` and call it "24H".
 * Null when no hour in the window has a value.
 */
export function precipitationNext24h(forecast: OpenMeteoResponse, now = Date.now()): number | null {
  const start = startHourIndex(forecast.hourly.time, now, forecast.utc_offset_seconds);
  const window = forecast.hourly.precipitation.slice(start, start + 24);
  const known = window.filter((v): v is number => v != null && Number.isFinite(v));
  if (known.length === 0) return null;
  return known.reduce((sum, v) => sum + Math.max(0, v), 0);
}

export interface DayView {
  date: string;
  isToday: boolean;
  hi: number | null;
  lo: number | null;
  icon: IconKind;
  chance: number | null;
}

export function dailyView(forecast: OpenMeteoResponse, unit: TemperatureUnit, now = new Date()): DayView[] {
  const today = wallClock(now, forecast.utc_offset_seconds).dateKey;
  const { daily } = forecast;
  return daily.time.map((t, i) => {
    const chance = daily.precipitation_probability_max?.[i];
    return {
      date: t,
      isToday: t === today,
      hi: displayTemperature(daily.temperature_2m_max[i], unit),
      lo: displayTemperature(daily.temperature_2m_min[i], unit),
      icon: getIconKind(daily.weather_code[i], false),
      chance: chance == null ? null : Math.round(chance),
    };
  });
}

/** Range of the week's known temperatures for the bar chart, or null if none. */
export function weekRange(days: DayView[]): { lo: number; hi: number } | null {
  const values = days.flatMap((d) => [d.lo, d.hi]).filter((v): v is number => v !== null);
  if (values.length === 0) return null;
  return { lo: Math.min(...values), hi: Math.max(...values) };
}

/**
 * "Rain starts in N min" from the 15-minute series. Only known samples count;
 * a gap is not treated as dry, so a window with no known values says nothing.
 */
export function nowcastHeadline(
  minutely: Minutely | undefined,
  now = Date.now(),
  utcOffsetSeconds?: number,
): { headline: string; sub: string } | null {
  if (!minutely?.precipitation?.length) return null;
  const startIdx = minutely.time.findIndex(
    (t) => (parseLocationTime(t, utcOffsetSeconds)?.getTime() ?? -Infinity) >= now - 7.5 * 60_000,
  );
  if (startIdx < 0) return null;
  const slice = minutely.precipitation.slice(startIdx, startIdx + 8);
  const firstWet = slice.findIndex((p) => p != null && p > 0.01);
  if (firstWet < 0) return null;
  const afterStart = slice.slice(firstWet);
  const dryAt = afterStart.findIndex((p) => p != null && p < 0.005);
  const lastsMin = (dryAt < 0 ? afterStart.length : dryAt) * 15;
  const total = afterStart.reduce<number>((sum, p) => sum + (p != null ? Math.max(0, p) : 0), 0);
  const heavy = afterStart.some((p) => p != null && p > 0.3);
  const minutes = firstWet * 15;
  return {
    headline: rainHeadline(heavy, minutes),
    sub: `Lasts ~${lastsMin} min · ${total.toFixed(2)}" total`,
  };
}

/** The banner's headline: the fact first, then a nudge. */
export function rainHeadline(heavy: boolean, startMinutes: number): string {
  if (startMinutes <= 0) return heavy ? "Heavy rain, right now" : "Rain's here";
  return heavy ? `Heavy rain in ${startMinutes} min. Heads up` : `Rain in ${startMinutes} min. Heads up`;
}

/** The radar nowcast when it has usable points; the Nowcast tab uses the same rule. */
export function usableRadarNowcast(radar: RadarNowcastResponse | undefined): RadarNowcastResponse | null {
  return radar && (radar.status === "ok" || radar.status === "degraded") && radar.points.length > 0 ? radar : null;
}

/**
 * The next-hour rain banner. Prefers the radar (pySTEPS) nowcast, like the
 * Nowcast tab, so the two screens can't disagree; falls back to the model's
 * 15-minute series only when the radar nowcast is unavailable.
 */
export function nextHourBanner(
  radar: RadarNowcastResponse | undefined,
  minutely: Minutely | undefined,
  now = Date.now(),
  utcOffsetSeconds?: number,
): { headline: string; sub: string } | null {
  const usable = usableRadarNowcast(radar);
  if (!usable) return nowcastHeadline(minutely, now, utcOffsetSeconds);
  const perMinute = interpolateRadarNowcast(usable.points);
  const verdict = getNowcastVerdict(perMinute);
  if (verdict.kind !== "raining" && verdict.kind !== "starting") return null;
  const start = verdict.kind === "starting" ? verdict.startMinute : 0;
  const heavy = perMinute.some((inchesPerHour) => inchesPerHour > 0.3);
  // The server scores every nowcast against the radar that arrives later;
  // say how it has been doing next to the claim, when there is enough to say.
  const skill = describeNowcastSkill(usable.skill);
  const source = skill ? `radar, ${skill.short}` : "from radar";
  return {
    headline: rainHeadline(heavy, start),
    sub: `Lasts ~${Math.max(1, verdict.endMinute - start)} min \u00B7 ${source}`,
  };
}
