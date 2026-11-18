/**
 * "Rain starts in N minutes" alerts planned on the phone from the radar
 * nowcast, delivered as local notifications. No push service is involved:
 * the app already holds the answer, so it schedules the notification itself.
 * Everything here is pure so it can be tested; scheduling lives in
 * rainAlertScheduler.ts.
 */
import type { PrecipClass, RadarNowcastPoint, RadarNowcastResponse } from "./nowcastTypes";

export type { PrecipClass } from "./nowcastTypes";
export type WinterClass = Exclude<PrecipClass, "rain">;

export const RAIN_ALERT_LEAD_OPTIONS = [5, 10, 15, 20] as const;
export type RainAlertLeadMinutes = (typeof RAIN_ALERT_LEAD_OPTIONS)[number];
export const DEFAULT_RAIN_ALERT_LEAD: RainAlertLeadMinutes = 10;

/** Inches per hour; matches the Nowcast screen's "rain" verdict threshold. */
export const RAIN_START_IN_HR = 0.08;
export const HEAVY_RAIN_IN_HR = 0.3;
/** Below this the event is over (same rule as the Nowcast screen's end). */
export const RAIN_END_IN_HR = 0.05;
/**
 * Snow start in inches/hour of the server's rain-rate scale: about 15 dBZ. Snow reads lower on
 * radar than rain for the same impact. Set from the 2025-01-06 Washington snow
 * (testdata/snowStart.kdca-2025-01-06.json): KDCA reported snow at 05:55 UTC as MRMS reached 15 dBZ,
 * after half an hour of 12-14 dBZ echoes that had not reached the ground. The rain start (about
 * 28 dBZ) never fired that night: the echo peaked near 20 dBZ in heavy snow.
 */
export const SNOW_START_IN_HR = 0.012;
/** Below this the snow is over (the rain end-to-start ratio). Mix uses the snow thresholds too. */
export const SNOW_END_IN_HR = 0.0075;
/** A second alert for what is plainly the same event is noise. */
export const SAME_EVENT_WINDOW_MS = 10 * 60_000;
export const RENOTIFY_AFTER_MS = 90 * 60_000;

export interface RainAlertPrefs {
  leadMinutes: number;
}

export interface RainAlertPlan {
  /** Stable id for this event; the same storm keeps the same key across refreshes. */
  key: string;
  /** Epoch ms when the notification should show. */
  fireAt: number;
  /** Epoch ms when rain is expected to start at the location. */
  startAt: number;
  /** Epoch ms when it is expected to end, or null if it lasts past the horizon. */
  endAt: number | null;
  peakInHr: number;
  heavy: boolean;
  title: string;
  body: string;
  /** Class at the start; rain when the server sent none. */
  precipClass?: PrecipClass;
}

export interface LastRainAlert {
  key: string;
  startAt: number;
  firedAt: number;
}

export type Sample = { at: number; inHr: number; cls?: PrecipClass };

const CLASSES: readonly PrecipClass[] = ["rain", "snow", "mix", "ice"];

function samples(points: RadarNowcastPoint[]): Sample[] {
  return points
    .map((point) => ({
      at: new Date(point.timestamp).getTime(),
      inHr: point.precipitation_mm_h / 25.4,
      cls: CLASSES.includes(point.precip_class as PrecipClass) ? point.precip_class : undefined,
    }))
    .filter((s) => Number.isFinite(s.at) && Number.isFinite(s.inHr))
    .sort((a, b) => a.at - b.at);
}

/** The class of the sample `at` moves toward (the first at or after it); rain when unknown. */
export function classAt(series: Sample[], at: number): PrecipClass {
  return (series.find((s) => s.at >= at) ?? series[series.length - 1])?.cls ?? "rain";
}

export function startThreshold(cls: PrecipClass): number {
  return cls === "snow" || cls === "mix" ? SNOW_START_IN_HR : RAIN_START_IN_HR;
}

export function endThreshold(cls: PrecipClass): number {
  return cls === "snow" || cls === "mix" ? SNOW_END_IN_HR : RAIN_END_IN_HR;
}

const WINTER_LEAD: Record<WinterClass, string> = {
  snow: "Snow starts in",
  mix: "Wintry mix starts in",
  ice: "Freezing rain or sleet starts in",
};
const WINTER_NOUN: Record<WinterClass, string> = {
  snow: "snow",
  mix: "a wintry mix",
  ice: "freezing rain or sleet",
};

/** "Snow starts in", "Wintry mix starts in" or "Freezing rain or sleet starts in". */
export function winterStartLead(cls: WinterClass): string {
  return WINTER_LEAD[cls];
}

/** "Snow starts in ~N min" and the like. */
export function winterStartWords(cls: WinterClass, minutes: number): string {
  return `${WINTER_LEAD[cls]} ~${Math.max(1, Math.round(minutes))} min`;
}

interface Start {
  cls: PrecipClass;
  startAt: number;
  endAt: number | null;
  peak: number;
}

/** The first minute the series crosses its class's start threshold, when it is dry now. */
function findStart(series: Sample[], now: number): Start | null {
  if (series.length < 2) return null;
  const horizonEnd = series[series.length - 1].at;
  if (horizonEnd <= now) return null;

  const current = intensityAt(series, now);
  if (current == null || current >= startThreshold(classAt(series, now))) return null; // precipitating already, or unknown

  let startAt: number | null = null;
  for (let t = now; t <= horizonEnd; t += 60_000) {
    const v = intensityAt(series, t);
    if (v != null && v >= startThreshold(classAt(series, t))) {
      startAt = t;
      break;
    }
  }
  if (startAt == null) return null;
  const cls = classAt(series, startAt);

  let endAt: number | null = null;
  let peak = 0;
  for (let t = startAt; t <= horizonEnd; t += 60_000) {
    const v = intensityAt(series, t) ?? 0;
    peak = Math.max(peak, v);
    if (v < endThreshold(cls)) {
      endAt = t;
      break;
    }
  }
  return { cls, startAt, endAt, peak };
}

export interface WinterStart {
  precipClass: WinterClass;
  startAt: number;
  endAt: number | null;
  startInMin: number;
  /** "Snow starts in ~N min" and the like. */
  words: string;
}

/** Snow, mix or ice starting within the nowcast hour, for the screens' headline. Null for rain. */
export function winterStart(nowcast: RadarNowcastResponse | null | undefined, now = Date.now()): WinterStart | null {
  if (!nowcast || (nowcast.status !== "ok" && nowcast.status !== "degraded")) return null;
  const start = findStart(samples(nowcast.points ?? []), now);
  if (!start || start.cls === "rain") return null;
  const startInMin = Math.max(1, Math.round((start.startAt - now) / 60_000));
  return {
    precipClass: start.cls,
    startAt: start.startAt,
    endAt: start.endAt,
    startInMin,
    words: winterStartWords(start.cls, startInMin),
  };
}

/**
 * Linear interpolation between the nowcast's absolute-time samples. Up to five
 * minutes before the first sample counts as "now"; past the last sample is unknown.
 */
export function intensityAt(series: Sample[], at: number): number | null {
  if (series.length === 0) return null;
  if (at <= series[0].at) return at < series[0].at - 5 * 60_000 ? null : series[0].inHr;
  const last = series[series.length - 1];
  if (at > last.at) return null;
  for (let i = 1; i < series.length; i++) {
    const a = series[i - 1];
    const b = series[i];
    if (at <= b.at) {
      const f = (at - a.at) / Math.max(1, b.at - a.at);
      return Math.max(0, a.inHr * (1 - f) + b.inHr * f);
    }
  }
  return last.inHr;
}

export function formatClock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/**
 * Decide whether the nowcast says rain will *start* at this location, and
 * when to say so. Null when it is already raining, stays dry, or the nowcast
 * is unusable: a notification is only worth sending for a change.
 */
export function planRainAlert(
  nowcast: RadarNowcastResponse | null | undefined,
  place: string,
  prefs: RainAlertPrefs,
  now = Date.now(),
): RainAlertPlan | null {
  if (!nowcast || (nowcast.status !== "ok" && nowcast.status !== "degraded")) return null;
  const start = findStart(samples(nowcast.points), now);
  if (!start) return null;
  const { cls, startAt, endAt, peak } = start;

  const heavy = cls === "rain" && peak >= HEAVY_RAIN_IN_HR;
  const startInMin = Math.max(1, Math.round((startAt - now) / 60_000));
  const durationMin = endAt == null ? null : Math.max(1, Math.round((endAt - startAt) / 60_000));
  const lead = Math.max(1, prefs.leadMinutes) * 60_000;
  const fireAt = Math.max(now + 1_000, startAt - lead);
  const kind = heavy ? "Heavy rain" : "Rain";
  const where = place.trim() || "your location";
  const lasting = durationMin == null ? "" : `, lasting about ${durationMin} min`;
  const key = `${nowcast.issued_at ?? "nowcast"}|${Math.round(startAt / SAME_EVENT_WINDOW_MS)}`;
  if (cls !== "rain") {
    // The title counts from when the notification shows.
    return {
      key,
      fireAt,
      startAt,
      endAt,
      peakInHr: peak,
      heavy,
      title: winterStartWords(cls, (startAt - fireAt) / 60_000),
      body: `Radar shows ${WINTER_NOUN[cls]} reaching ${where} around ${formatClock(startAt)}${lasting}.`,
      precipClass: cls,
    };
  }
  return {
    key,
    fireAt,
    startAt,
    endAt,
    peakInHr: peak,
    heavy,
    title: `${kind} starting around ${formatClock(startAt)}`,
    body: `Radar shows ${kind.toLowerCase()} reaching ${where} in about ${startInMin} min${lasting}.`,
  };
}

/**
 * Whether a plan is news: not the same event we already announced within the
 * last hour and a half. Refreshes every minute would otherwise re-fire.
 */
export function shouldNotify(plan: RainAlertPlan, last: LastRainAlert | null, now = Date.now()): boolean {
  if (!last) return true;
  const sameEvent = Math.abs(plan.startAt - last.startAt) <= SAME_EVENT_WINDOW_MS;
  const recent = now - last.firedAt <= RENOTIFY_AFTER_MS;
  return !(sameEvent && recent);
}

export function parseLastRainAlert(raw: string): LastRainAlert | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<LastRainAlert>;
    if (
      typeof parsed.key === "string" &&
      typeof parsed.startAt === "number" &&
      typeof parsed.firedAt === "number"
    ) {
      return { key: parsed.key, startAt: parsed.startAt, firedAt: parsed.firedAt };
    }
  } catch {}
  return null;
}

export function parseRainAlertLead(
  v: string,
  fallback: RainAlertLeadMinutes = DEFAULT_RAIN_ALERT_LEAD,
): RainAlertLeadMinutes {
  const n = Number(v);
  return (RAIN_ALERT_LEAD_OPTIONS as readonly number[]).includes(n) ? (n as RainAlertLeadMinutes) : fallback;
}
