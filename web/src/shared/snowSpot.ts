/**
 * "Snow at your spot": the `/api/snow/{lat}/{lon}` answer turned into card text and bars, shared
 * by the web forecast panel and the app Home tab. Every field is optional: a missing source drops
 * its line, never the card.
 */
import { forecastTimeMs } from "./forecastTime";

export interface SnowChance {
  inches: number;
  pct: number;
  windowEnd: number;
}

export interface SnowSpot {
  /** Hour stamps (ms); each value is the snow in the hour ending there, in inches. */
  hourly?: { time: number[]; snowfallIn: (number | null)[] };
  start?: number;
  end?: number;
  onGroundIn?: number;
  onGroundAt?: number;
  past24hIn?: number;
  past24hAt?: number;
  nbm24hIn?: number;
  nbm48hIn?: number;
  nbm72hIn?: number;
  ice72hIn?: number;
  nbmIssuedAt?: number;
  chance: SnowChance[];
  sources: string[];
}

export interface SnowClock {
  timeZone?: string;
  utcOffsetSeconds?: number;
}

/** Hours of bars on the card. */
export const SNOW_BAR_HOURS = 48;
/** The card shows when this much snow is due in the next 72 h ... */
export const SNOW_SHOW_FORECAST_IN = 0.1;
/** ... or this much is on the ground. */
export const SNOW_SHOW_GROUND_IN = 0.5;
/** NBM and Open-Meteo totals this close read as one number. */
export const SNOW_AGREE_IN = 0.5;
/** Open-Meteo weather codes for snow, used to reserve the card's space before the answer lands. */
const SNOW_CODES = new Set([71, 73, 75, 77, 85, 86]);
const HOUR_MS = 3_600_000;

const SOURCE_NAMES: Record<string, string> = {
  "open-meteo": "Open-Meteo",
  nbm: "NOAA NBM",
  snodas: "NOAA SNODAS",
  nohrsc: "NOAA NOHRSC",
  wpc: "NOAA WPC",
};

/** Same 0.1° cell the server caches on. */
export function snowSpotPath(lat: number, lon: number): string {
  return `/api/snow/${lat.toFixed(1)}/${lon.toFixed(1)}`;
}

const num = (v: unknown): number | undefined => (typeof v === "number" && Number.isFinite(v) ? v : undefined);
const time = (v: unknown): number | undefined => {
  if (typeof v !== "string") return undefined;
  const t = Date.parse(v);
  return Number.isFinite(t) ? t : undefined;
};

/** Tolerant parse: anything malformed is dropped field by field. `null` only for a non-object. */
export function parseSnowSpot(body: unknown): SnowSpot | null {
  if (!body || typeof body !== "object" || Array.isArray(body)) return null;
  const b = body as Record<string, unknown>;
  const out: SnowSpot = { chance: [], sources: [] };
  const h = b.hourly as { time?: unknown; snowfall_in?: unknown } | undefined;
  if (h && Array.isArray(h.time) && Array.isArray(h.snowfall_in) && h.time.length === h.snowfall_in.length) {
    const times = h.time.map(time);
    if (times.every((t) => t !== undefined)) {
      out.hourly = { time: times as number[], snowfallIn: h.snowfall_in.map((v) => num(v) ?? null) };
    }
  }
  out.start = time(b.start);
  out.end = time(b.end);
  out.onGroundIn = num(b.on_ground_in);
  out.onGroundAt = time(b.on_ground_at);
  out.past24hIn = num(b.past_24h_in);
  out.past24hAt = time(b.past_24h_at);
  out.nbm24hIn = num(b.nbm_24h_in);
  out.nbm48hIn = num(b.nbm_48h_in);
  out.nbm72hIn = num(b.nbm_72h_in);
  out.ice72hIn = num(b.ice_72h_in);
  out.nbmIssuedAt = time(b.nbm_issued_at);
  if (Array.isArray(b.chance)) {
    for (const c of b.chance as Record<string, unknown>[]) {
      const inches = num(c?.inches);
      const pct = num(c?.pct);
      const windowEnd = time(c?.window_end);
      if (inches !== undefined && pct !== undefined && windowEnd !== undefined) out.chance.push({ inches, pct, windowEnd });
    }
  }
  if (Array.isArray(b.sources)) out.sources = b.sources.filter((s): s is string => typeof s === "string");
  return out;
}

/** Open-Meteo snow total from now through `untilMs` (hours ending after now). */
function hourlyTotal(spot: SnowSpot, nowMs: number, untilMs: number): number | undefined {
  if (!spot.hourly) return undefined;
  let sum = 0;
  spot.hourly.time.forEach((t, i) => {
    if (t > nowMs && t <= untilMs) sum += spot.hourly!.snowfallIn[i] ?? 0;
  });
  return sum;
}

/** The NBM total whose horizon covers `untilMs`, else the longest one we have. */
function nbmTotal(spot: SnowSpot, nowMs: number, untilMs?: number): { inches: number; hours: number } | undefined {
  const all = [
    [24, spot.nbm24hIn],
    [48, spot.nbm48hIn],
    [72, spot.nbm72hIn],
  ].filter((p): p is [number, number] => p[1] !== undefined);
  if (!all.length) return undefined;
  const hit = untilMs === undefined ? undefined : all.find(([h]) => nowMs + h * HOUR_MS >= untilMs);
  const [hours, inches] = hit ?? all[all.length - 1];
  return { inches, hours };
}

/** Snow is due in the next 72 h or lying on the ground. */
export function snowCardShows(spot: SnowSpot | null, nowMs: number): boolean {
  if (!spot) return false;
  const om = hourlyTotal(spot, nowMs, nowMs + 72 * HOUR_MS) ?? 0;
  const nbm = Math.max(spot.nbm24hIn ?? 0, spot.nbm48hIn ?? 0, spot.nbm72hIn ?? 0);
  return Math.max(om, nbm) >= SNOW_SHOW_FORECAST_IN || (spot.onGroundIn ?? 0) >= SNOW_SHOW_GROUND_IN;
}

/** The local forecast calls for snow in the next 72 h: reserve the card's space while it loads. */
export function snowLikely(hourly: { time: string[]; weather_code?: (number | null)[] } | null | undefined, nowMs: number, utcOffsetSeconds?: number): boolean {
  if (!hourly?.weather_code) return false;
  return hourly.time.some((t, i) => {
    const at = forecastTimeMs(t, utcOffsetSeconds);
    return at > nowMs - HOUR_MS && at <= nowMs + 72 * HOUR_MS && SNOW_CODES.has(hourly.weather_code![i] ?? -1);
  });
}

/** "6.2 in", "12 in", "under 0.1 in" for measured amounts. */
export function inches(v: number): string {
  if (v < 0.1) return "under 0.1 in";
  return v < 10 ? `${Number(v.toFixed(1))} in` : `${Math.round(v)} in`;
}

const numberOnly = (v: number) => (v < 1 ? (Math.round(v * 10) / 10).toFixed(1) : String(Math.round(v)));

/** "3–5 in", or one number when the two totals agree within half an inch (or round the same). */
export function snowRange(a: number | undefined, b: number | undefined): string | undefined {
  const vals = [a, b].filter((v): v is number => v !== undefined);
  if (!vals.length) return undefined;
  const lo = Math.min(...vals);
  const hi = Math.max(...vals);
  if (hi < SNOW_SHOW_FORECAST_IN) return undefined;
  if (hi - lo <= SNOW_AGREE_IN || numberOnly(lo) === numberOnly(hi)) return `${numberOnly((lo + hi) / 2)} in`;
  return `${numberOnly(Math.max(lo, 0.1))}–${numberOnly(hi)} in`;
}

function parts(ms: number, c: SnowClock): { day: string; dateKey: string; hour: string } {
  if (c.timeZone) {
    try {
      const f = new Intl.DateTimeFormat("en-US", { timeZone: c.timeZone, weekday: "long", year: "numeric", month: "2-digit", day: "2-digit", hour: "numeric", hourCycle: "h23" });
      const p = Object.fromEntries(f.formatToParts(new Date(ms)).map((x) => [x.type, x.value]));
      return { day: p.weekday, dateKey: `${p.year}-${p.month}-${p.day}`, hour: hourWord(Number(p.hour)) };
    } catch {
      // Unknown zone name: fall through to the offset.
    }
  }
  const at = new Date(ms + (c.utcOffsetSeconds ?? 0) * 1000);
  const day = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][at.getUTCDay()];
  return { day, dateKey: at.toISOString().slice(0, 10), hour: hourWord(at.getUTCHours()) };
}

const hourWord = (h: number) => (h === 0 ? "midnight" : h === 12 ? "noon" : `${h % 12} ${h < 12 ? "am" : "pm"}`);

/** "7 am" today, else "Thursday 7 am". */
export function snowWhen(ms: number, nowMs: number, c: SnowClock): string {
  const p = parts(ms, c);
  return p.dateKey === parts(nowMs, c).dateKey ? p.hour : `${p.day} ${p.hour}`;
}

export interface SnowBar {
  at: number;
  inches: number;
  /** 0..1 of the bar area. */
  height: number;
}

export interface SnowSpotView {
  headline: string;
  /** "Starts 9 pm · ends Thursday 7 am", when the hourly block has snow. */
  window?: string;
  bars: SnowBar[];
  /** Up to four short lines; missing sources drop theirs. */
  lines: string[];
  credit: string;
}

/** Card text and bars. Works with any subset of fields; `empty` still gives a headline. */
export function snowSpotView(spot: SnowSpot, nowMs: number, clock: SnowClock): SnowSpotView {
  const end = spot.end !== undefined && spot.end > nowMs ? spot.end : undefined;
  const nbm = nbmTotal(spot, nowMs, end);
  const om = hourlyTotal(spot, nowMs, end ?? nowMs + 72 * HOUR_MS);
  const range = snowRange(nbm?.inches, om !== undefined && om > 0 ? om : undefined);
  const by = end ?? (nbm ? nowMs + nbm.hours * HOUR_MS : undefined);
  let headline: string;
  if (range && by !== undefined) headline = `${range} by ${snowWhen(by, nowMs, clock)}`;
  else if (range) headline = `${range} in the next 72 h`;
  else if ((spot.onGroundIn ?? 0) >= SNOW_SHOW_GROUND_IN) headline = `${inches(spot.onGroundIn!)} on the ground`;
  else headline = "No snow expected";

  let window: string | undefined;
  if (spot.start !== undefined && end !== undefined) {
    window = spot.start > nowMs ? `Starts ${snowWhen(spot.start, nowMs, clock)} · ends ${snowWhen(end, nowMs, clock)}` : `Snowing now · ends ${snowWhen(end, nowMs, clock)}`;
  }

  const bars: SnowBar[] = [];
  if (spot.hourly) {
    spot.hourly.time.forEach((at, i) => {
      if (at > nowMs && bars.length < SNOW_BAR_HOURS) bars.push({ at, inches: spot.hourly!.snowfallIn[i] ?? 0, height: 0 });
    });
    const top = Math.max(0.5, ...bars.map((b) => b.inches));
    for (const b of bars) b.height = Math.min(1, b.inches / top);
  }

  const lines: string[] = [];
  if (spot.onGroundIn !== undefined) lines.push(spot.onGroundIn >= 0.1 ? `${inches(spot.onGroundIn)} on the ground` : "No snow on the ground");
  if (spot.past24hIn !== undefined) lines.push(spot.past24hIn >= 0.1 ? `${inches(spot.past24hIn)} fell in the last 24 h` : "No snow in the last 24 h");
  if (spot.ice72hIn !== undefined && spot.ice72hIn >= 0.01) lines.push(`Ice up to ${spot.ice72hIn.toFixed(2)} in`);
  const four = spot.chance.find((c) => c.inches === 4);
  if (four) lines.push(`${Math.round(four.pct)}% chance of 4 in or more`);

  const credit = spot.sources.map((s) => SOURCE_NAMES[s] ?? s).join(" · ");
  return { headline, window, bars, lines: lines.slice(0, 4), credit };
}

/** One sentence for screen readers. */
export function snowSpotLabel(v: SnowSpotView): string {
  return [v.headline, v.window, ...v.lines].filter(Boolean).join(". ");
}

/** Small LRU capped by JSON size, so a long session of place changes cannot grow without bound. */
export class ByteCappedCache<V> {
  private map = new Map<string, { value: V; bytes: number; at: number }>();
  private total = 0;
  constructor(private readonly maxBytes: number, private readonly ttlMs: number) {}
  get(key: string, nowMs = Date.now()): V | undefined {
    const hit = this.map.get(key);
    if (!hit) return undefined;
    if (nowMs - hit.at > this.ttlMs) {
      this.delete(key);
      return undefined;
    }
    this.map.delete(key);
    this.map.set(key, hit);
    return hit.value;
  }
  set(key: string, value: V, bytes: number, nowMs = Date.now()): void {
    this.delete(key);
    if (bytes > this.maxBytes) return;
    this.map.set(key, { value, bytes, at: nowMs });
    this.total += bytes;
    for (const [k] of this.map) {
      if (this.total <= this.maxBytes) break;
      this.delete(k);
    }
  }
  get bytes(): number {
    return this.total;
  }
  private delete(key: string) {
    const hit = this.map.get(key);
    if (hit) this.total -= hit.bytes;
    this.map.delete(key);
  }
}
