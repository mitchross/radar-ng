/**
 * Pure derivations for the forecast screens. Every value can be missing in the
 * source (Open-Meteo returns null past a model's horizon), so each result
 * carries null through instead of inventing 0°, 0%, clear skies or a
 * "normal" pressure.
 */
import { getIconKind, type IconKind } from "./cumulusTheme";
import { displayTemperature } from "./temperature";
import type { OpenMeteoResponse, TemperatureUnit } from "../types/weather";

type Daily = OpenMeteoResponse["daily"];
type Minutely = NonNullable<OpenMeteoResponse["minutely_15"]>;

function parseTime(value: string | null | undefined): Date | null {
  if (!value) return null;
  const d = new Date(value);
  return Number.isFinite(d.getTime()) ? d : null;
}

/** Sunrise/sunset for the local calendar day that contains `at`, or nulls. */
export function sunTimesFor(daily: Daily, at: Date): { sunrise: Date | null; sunset: Date | null } {
  const key = `${at.getFullYear()}-${String(at.getMonth() + 1).padStart(2, "0")}-${String(at.getDate()).padStart(2, "0")}`;
  const index = daily.time.indexOf(key);
  if (index < 0) return { sunrise: null, sunset: null };
  return { sunrise: parseTime(daily.sunrise[index]), sunset: parseTime(daily.sunset[index]) };
}

/**
 * Night for the hour's own day, not today's. Tomorrow 8 AM compared against
 * today's sunset used to read as night. Without sun times, fall back to a
 * fixed 06:00–20:00 day rather than guessing from today.
 */
export function isNightAt(at: Date, daily: Daily): boolean {
  const { sunrise, sunset } = sunTimesFor(daily, at);
  if (sunrise && sunset) return at < sunrise || at > sunset;
  const hour = at.getHours();
  return hour < 6 || hour >= 20;
}

/** Index of the first hourly entry at or after "now" (with a 30-minute grace). */
export function startHourIndex(hours: string[], now = Date.now()): number {
  for (let i = 0; i < hours.length; i++) {
    const t = parseTime(hours[i]);
    if (t && t.getTime() >= now - 30 * 60_000) return i;
  }
  return 0;
}

function formatHour(d: Date, i: number): string {
  if (i === 0) return "NOW";
  const h = d.getHours();
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
  const { hourly, daily } = forecast;
  const start = startHourIndex(hourly.time, now);
  return hourly.time.slice(start, start + count).map((t, i) => {
    const idx = start + i;
    const at = parseTime(t) ?? new Date(now);
    const night = isNightAt(at, daily);
    return {
      label: formatHour(at, i),
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
  const start = startHourIndex(forecast.hourly.time, now);
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
  const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
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
): { headline: string; sub: string } | null {
  if (!minutely?.precipitation?.length) return null;
  const startIdx = minutely.time.findIndex((t) => (parseTime(t)?.getTime() ?? -Infinity) >= now - 7.5 * 60_000);
  if (startIdx < 0) return null;
  const slice = minutely.precipitation.slice(startIdx, startIdx + 8);
  const firstWet = slice.findIndex((p) => p != null && p > 0.01);
  if (firstWet < 0) return null;
  const afterStart = slice.slice(firstWet);
  const dryAt = afterStart.findIndex((p) => p != null && p < 0.005);
  const lastsMin = (dryAt < 0 ? afterStart.length : dryAt) * 15;
  const total = afterStart.reduce<number>((sum, p) => sum + (p != null ? Math.max(0, p) : 0), 0);
  const heavy = afterStart.some((p) => p != null && p > 0.3);
  const kind = heavy ? "Heavy rain" : "Rain";
  const minutes = firstWet * 15;
  return {
    headline: minutes === 0 ? `${kind} now` : `${kind} starts in ${minutes} min`,
    sub: `Lasts ~${lastsMin} min · ${total.toFixed(2)}" total`,
  };
}
