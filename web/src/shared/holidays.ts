/**
 * US holiday table for the countdown card: federal holidays by rule, plus
 * Easter, Halloween and the clock changes. No data source. Dates are the
 * day itself, not the "observed" weekday off.
 */
import { clockLabel, sunTimes } from "./astro";

export type HolidayKind = "federal" | "observance" | "clock";

export interface Holiday {
  id: string;
  name: string;
  /** "YYYY-MM-DD" calendar date at the location. */
  dateKey: string;
  kind: HolidayKind;
}

const DAY_MS = 86_400_000;
const pad = (n: number) => String(n).padStart(2, "0");
const key = (y: number, m: number, d: number) => `${y}-${pad(m)}-${pad(d)}`;

/** Day of month of the nth `weekday` (0 = Sunday) in `month` (1–12); n = -1 is the last. */
export function nthWeekday(year: number, month: number, weekday: number, n: number): number {
  if (n < 0) {
    const last = new Date(Date.UTC(year, month, 0));
    return last.getUTCDate() - ((last.getUTCDay() - weekday + 7) % 7);
  }
  const first = new Date(Date.UTC(year, month - 1, 1)).getUTCDay();
  return 1 + ((weekday - first + 7) % 7) + (n - 1) * 7;
}

/** Western (Gregorian) Easter Sunday, anonymous Gregorian algorithm. */
export function easter(year: number): { month: number; day: number } {
  const a = year % 19;
  const b = Math.floor(year / 100);
  const c = year % 100;
  const d = Math.floor(b / 4);
  const e = b % 4;
  const f = Math.floor((b + 8) / 25);
  const g = Math.floor((b - f + 1) / 3);
  const h = (19 * a + b - d - g + 15) % 30;
  const i = Math.floor(c / 4);
  const k = c % 4;
  const l = (32 + 2 * e + 2 * i - h - k) % 7;
  const m = Math.floor((a + 11 * h + 22 * l) / 451);
  const month = Math.floor((h + l - 7 * m + 114) / 31);
  return { month, day: ((h + l - 7 * m + 114) % 31) + 1 };
}

/** Every holiday in `year`, in date order. */
export function holidaysForYear(year: number): Holiday[] {
  const e = easter(year);
  const list: Holiday[] = [
    { id: "new-year", name: "New Year's Day", dateKey: key(year, 1, 1), kind: "federal" },
    { id: "mlk", name: "Martin Luther King Jr. Day", dateKey: key(year, 1, nthWeekday(year, 1, 1, 3)), kind: "federal" },
    { id: "presidents", name: "Presidents' Day", dateKey: key(year, 2, nthWeekday(year, 2, 1, 3)), kind: "federal" },
    { id: "dst-start", name: "Clocks go forward", dateKey: key(year, 3, nthWeekday(year, 3, 0, 2)), kind: "clock" },
    { id: "easter", name: "Easter", dateKey: key(year, e.month, e.day), kind: "observance" },
    { id: "memorial", name: "Memorial Day", dateKey: key(year, 5, nthWeekday(year, 5, 1, -1)), kind: "federal" },
    { id: "juneteenth", name: "Juneteenth", dateKey: key(year, 6, 19), kind: "federal" },
    { id: "independence", name: "Independence Day", dateKey: key(year, 7, 4), kind: "federal" },
    { id: "labor", name: "Labor Day", dateKey: key(year, 9, nthWeekday(year, 9, 1, 1)), kind: "federal" },
    { id: "columbus", name: "Columbus Day", dateKey: key(year, 10, nthWeekday(year, 10, 1, 2)), kind: "federal" },
    { id: "halloween", name: "Halloween", dateKey: key(year, 10, 31), kind: "observance" },
    { id: "dst-end", name: "Clocks go back", dateKey: key(year, 11, nthWeekday(year, 11, 0, 1)), kind: "clock" },
    { id: "veterans", name: "Veterans Day", dateKey: key(year, 11, 11), kind: "federal" },
    { id: "thanksgiving", name: "Thanksgiving", dateKey: key(year, 11, nthWeekday(year, 11, 4, 4)), kind: "federal" },
    { id: "christmas", name: "Christmas", dateKey: key(year, 12, 25), kind: "federal" },
  ];
  return list.sort((x, y) => (x.dateKey < y.dateKey ? -1 : x.dateKey > y.dateKey ? 1 : 0));
}

const keyMs = (dateKey: string) => Date.UTC(+dateKey.slice(0, 4), +dateKey.slice(5, 7) - 1, +dateKey.slice(8, 10));

/** Whole calendar days from `fromKey` to `toKey`. */
export function daysBetween(fromKey: string, toKey: string): number {
  return Math.round((keyMs(toKey) - keyMs(fromKey)) / DAY_MS);
}

/**
 * False when the zone keeps one offset all year (Arizona, Hawaii), so the
 * card skips the clock changes there. True when unknown.
 */
export function zoneObservesDst(timeZone?: string): boolean {
  if (!timeZone) return true;
  try {
    const f = new Intl.DateTimeFormat("en-US", { timeZone, hour: "numeric", hour12: false });
    const year = new Date().getUTCFullYear();
    return f.format(new Date(Date.UTC(year, 0, 15, 12))) !== f.format(new Date(Date.UTC(year, 6, 15, 12)));
  } catch {
    return true;
  }
}

/** The next `count` holidays on or after `todayKey`, crossing into next year when needed. */
export function upcomingHolidays(todayKey: string, count = 3, opts: { clockChanges?: boolean } = {}): Holiday[] {
  const year = +todayKey.slice(0, 4);
  return [...holidaysForYear(year), ...holidaysForYear(year + 1)]
    .filter((h) => h.dateKey >= todayKey && (opts.clockChanges !== false || h.kind !== "clock"))
    .slice(0, count);
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export interface CountdownRow {
  id: string;
  name: string;
  /** "Thu, Nov 26" */
  dateLabel: string;
  /** "Today", "Tomorrow" or "in 9 days" */
  inLabel: string;
  /** "Sunset 5:01 PM", or null when the sun does not set that day. */
  sunsetLabel: string | null;
}

export interface CountdownPlace {
  lat: number;
  lon: number;
  /** IANA zone from the forecast ("America/Detroit"). */
  timeZone?: string;
  utcOffsetSeconds?: number;
}

/** Rows for the countdown card: the next holidays with the sunset on each. */
export function countdownRows(todayKey: string, place: CountdownPlace, count = 3): CountdownRow[] {
  return upcomingHolidays(todayKey, count, { clockChanges: zoneObservesDst(place.timeZone) }).map((h) => {
    const days = daysBetween(todayKey, h.dateKey);
    const d = new Date(keyMs(h.dateKey));
    const sunset = sunTimes(h.dateKey, place.lat, place.lon).set;
    return {
      id: h.id,
      name: h.name,
      dateLabel: `${WEEKDAYS[d.getUTCDay()]}, ${MONTHS[d.getUTCMonth()]} ${d.getUTCDate()}`,
      inLabel: days === 0 ? "Today" : days === 1 ? "Tomorrow" : `in ${days} days`,
      sunsetLabel: sunset == null ? null : `Sunset ${clockLabel(sunset, place.timeZone, place.utcOffsetSeconds)}`,
    };
  });
}
