/**
 * Nice Day and Bonfire index: our own score for one forecast hour, not an
 * official index. Pure, shared by the web panel and the app (and mirrored by
 * the Rust `nice-day` layer, which reads the same testdata/niceDay.vectors.json).
 * Units are the forecast API's: °F, mph, %, inches per hour.
 */

export type NiceDayMode = "niceDay" | "bonfire";
export type NiceDayWord = "Great" | "Good" | "Fair" | "Poor" | "Do Not";
export type NiceDayFactor = "temp" | "wind" | "rainChance" | "rainAmount" | "humidity";

/** One forecast hour; any field may be missing. */
export interface NiceDayHour {
  tempF?: number | null;
  feelsF?: number | null;
  windMph?: number | null;
  rainChance?: number | null;
  rainIn?: number | null;
  humidity?: number | null;
}

export interface NiceDayScore {
  score: number;
  word: NiceDayWord;
}

/** [input, points off] pairs, linear between them, flat past the ends. */
type Ramp = readonly (readonly [number, number])[];

interface ModeTable {
  /** Points taken off 100 per factor. A missing optional factor takes nothing off. */
  penalties: Readonly<Partial<Record<NiceDayFactor, Ramp>>>;
  /** Humidity only counts when the hour feels at least this warm (°F). */
  humidAboveF: number;
  /** Sustained wind above this (mph) is always Do Not. */
  maxWindMph: number;
  /** Factors without which the hour is not scored. */
  required: readonly NiceDayFactor[];
}

/** Every threshold in one place. Change a number here and in the vectors together. */
export const NICE_DAY_TABLE: Readonly<Record<NiceDayMode, ModeTable>> = {
  niceDay: {
    penalties: {
      temp: [[0, 100], [25, 65], [40, 40], [50, 20], [62, 0], [78, 0], [85, 25], [92, 55], [100, 100]],
      wind: [[10, 0], [15, 10], [20, 25], [30, 50], [40, 80]],
      rainChance: [[20, 0], [50, 15], [80, 30], [100, 40]],
      rainAmount: [[0, 0], [0.01, 20], [0.05, 40], [0.1, 55], [0.3, 90]],
      humidity: [[60, 0], [75, 10], [90, 25], [100, 35]],
    },
    humidAboveF: 70,
    maxWindMph: Infinity,
    required: ["temp"],
  },
  // Evening weights: light wind matters most, then dry, then cool but not cold.
  bonfire: {
    penalties: {
      wind: [[5, 0], [10, 15], [15, 35], [20, 60]],
      rainChance: [[10, 0], [40, 25], [70, 50], [100, 70]],
      rainAmount: [[0, 0], [0.01, 40], [0.05, 80], [0.1, 100]],
      temp: [[0, 100], [20, 55], [32, 25], [42, 0], [65, 0], [75, 20], [85, 45], [95, 70]],
    },
    humidAboveF: Infinity,
    maxWindMph: 20,
    required: ["temp", "wind"],
  },
};

/** Lower bound of each word, best first. */
export const NICE_DAY_WORDS: readonly (readonly [number, NiceDayWord])[] = [
  [80, "Great"],
  [60, "Good"],
  [40, "Fair"],
  [20, "Poor"],
  [0, "Do Not"],
];

export const NICE_DAY_CREDIT = "Our own index from your forecast, not an official one.";

const num = (v: number | null | undefined): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

function ramp(points: Ramp, x: number): number {
  if (x <= points[0][0]) return points[0][1];
  for (let i = 1; i < points.length; i++) {
    const [x1, y1] = points[i];
    if (x <= x1) {
      const [x0, y0] = points[i - 1];
      return y0 + ((y1 - y0) * (x - x0)) / (x1 - x0);
    }
  }
  return points[points.length - 1][1];
}

export function wordFor(score: number): NiceDayWord {
  return NICE_DAY_WORDS.find(([min]) => score >= min)?.[1] ?? "Do Not";
}

/**
 * Score one hour: 100 minus each factor's points off, rounded, floored at 0.
 * Feels-like wins over air temperature. A missing required factor
 * (temperature; wind for Bonfire) gives null rather than a guess.
 */
export function scoreHour(hour: NiceDayHour, mode: NiceDayMode = "niceDay"): NiceDayScore | null {
  const t = NICE_DAY_TABLE[mode];
  const temp = num(hour.feelsF) ?? num(hour.tempF);
  const inputs: Record<NiceDayFactor, number | null> = {
    temp,
    wind: num(hour.windMph),
    rainChance: num(hour.rainChance),
    rainAmount: num(hour.rainIn),
    humidity: temp != null && temp >= t.humidAboveF ? num(hour.humidity) : null,
  };
  if (t.required.some((f) => inputs[f] == null)) return null;
  if (inputs.wind != null && inputs.wind > t.maxWindMph) return { score: 0, word: "Do Not" };
  let off = 0;
  for (const f of Object.keys(inputs) as NiceDayFactor[]) {
    const x = inputs[f];
    const p = t.penalties[f];
    if (x != null && p) off += ramp(p, x);
  }
  const score = Math.max(0, Math.round(100 - off));
  return { score, word: wordFor(score) };
}

// ---------- card summary: today's best hours and tonight's Bonfire ----------

/** The Open-Meteo hourly block as both clients receive it. */
export interface NiceDayHourly {
  time: readonly string[];
  temperature_2m?: readonly (number | null)[];
  apparent_temperature?: readonly (number | null)[];
  wind_speed_10m?: readonly (number | null)[];
  precipitation_probability?: readonly (number | null)[];
  precipitation?: readonly (number | null)[];
  relative_humidity_2m?: readonly (number | null)[];
}

export interface NiceDayWindow {
  word: NiceDayWord;
  score: number;
  /** Wall-clock hours at the location: first hour of the run, and the hour it ends (exclusive). */
  startHour: number;
  endHour: number;
  /** True when the window is on the next calendar day. */
  tomorrow: boolean;
}

export interface NiceDaySummary {
  best: NiceDayWindow | null;
  bonfire: NiceDayWindow | null;
}

/** Daytime hours scored for "best hours", and evening hours for Bonfire (wall clock, end exclusive). */
export const NICE_DAY_HOURS = { day: [8, 20], evening: [18, 24] } as const;

const WALL = /^(\d{4}-\d{2}-\d{2})T(\d{2})/;

export function hourAt(h: NiceDayHourly, i: number): NiceDayHour {
  return {
    tempF: h.temperature_2m?.[i],
    feelsF: h.apparent_temperature?.[i],
    windMph: h.wind_speed_10m?.[i],
    rainChance: h.precipitation_probability?.[i],
    rainIn: h.precipitation?.[i],
    humidity: h.relative_humidity_2m?.[i],
  };
}

/**
 * Best run on the first calendar day (from `start`, the current hour's index)
 * that still has hours in the [from, to) wall-clock window: the top-scoring
 * hour, widened over adjacent hours with the same word.
 */
function bestWindow(h: NiceDayHourly, start: number, todayKey: string, mode: NiceDayMode, [from, to]: readonly [number, number]): NiceDayWindow | null {
  const slots: { day: string; hour: number; s: NiceDayScore | null }[] = [];
  for (let i = Math.max(0, start); i < h.time.length; i++) {
    const m = WALL.exec(h.time[i] ?? "");
    if (!m) continue;
    const hour = +m[2];
    if (hour < from || hour >= to) continue;
    if (slots.length && m[1] !== slots[0].day) break;
    slots.push({ day: m[1], hour, s: scoreHour(hourAt(h, i), mode) });
  }
  let top = -1;
  slots.forEach((slot, k) => {
    if (slot.s && (top < 0 || slot.s.score > slots[top].s!.score)) top = k;
  });
  if (top < 0) return null;
  const { word, score } = slots[top].s!;
  let a = top;
  let b = top;
  while (a > 0 && slots[a - 1].s?.word === word && slots[a].hour - slots[a - 1].hour === 1) a--;
  while (b < slots.length - 1 && slots[b + 1].s?.word === word && slots[b + 1].hour - slots[b].hour === 1) b++;
  return { word, score, startHour: slots[a].hour, endHour: slots[b].hour + 1, tomorrow: slots[top].day !== todayKey };
}

/**
 * Card data from the forecast already loaded. `start` is the index of the
 * current hour and `todayKey` the location's date ("2026-11-17", from
 * `wallClock` in forecastTime).
 */
export function niceDaySummary(h: NiceDayHourly, start: number, todayKey: string): NiceDaySummary {
  return {
    best: bestWindow(h, start, todayKey, "niceDay", NICE_DAY_HOURS.day),
    bonfire: bestWindow(h, start, todayKey, "bonfire", NICE_DAY_HOURS.evening),
  };
}

/** 0–24 wall-clock hour → "1 PM", "12 AM". */
export function hourLabel(hour: number): string {
  const h = ((hour % 24) + 24) % 24;
  return `${h % 12 || 12} ${h < 12 ? "AM" : "PM"}`;
}

/** "1 PM–4 PM", or "1 PM" for a single hour. */
export function windowLabel(w: NiceDayWindow): string {
  return w.endHour - w.startHour <= 1 ? hourLabel(w.startHour) : `${hourLabel(w.startHour)}–${hourLabel(w.endHour)}`;
}

export interface NiceDayRow {
  label: string;
  word: NiceDayWord;
  when: string;
}

/** The card rows both clients show, in order; a row without data is left out. */
export function niceDayRows(s: NiceDaySummary): NiceDayRow[] {
  const rows: NiceDayRow[] = [];
  if (s.best) rows.push({ label: s.best.tomorrow ? "Nice Day tomorrow" : "Nice Day today", word: s.best.word, when: `Best ${windowLabel(s.best)}` });
  if (s.bonfire) rows.push({ label: s.bonfire.tomorrow ? "Bonfire tomorrow night" : "Bonfire tonight", word: s.bonfire.word, when: `Best ${windowLabel(s.bonfire)}` });
  return rows;
}
