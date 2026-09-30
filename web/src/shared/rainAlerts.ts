/**
 * "Rain starts in N minutes" alerts planned on the phone from the radar
 * nowcast, delivered as local notifications. No push service is involved:
 * the app already holds the answer, so it schedules the notification itself.
 * Everything here is pure so it can be tested; scheduling lives in
 * rainAlertScheduler.ts.
 */
import type { RadarNowcastPoint, RadarNowcastResponse } from "./nowcastTypes";

export const RAIN_ALERT_LEAD_OPTIONS = [5, 10, 15, 20] as const;
export type RainAlertLeadMinutes = (typeof RAIN_ALERT_LEAD_OPTIONS)[number];
export const DEFAULT_RAIN_ALERT_LEAD: RainAlertLeadMinutes = 10;

/** Inches per hour; matches the Nowcast screen's "rain" verdict threshold. */
export const RAIN_START_IN_HR = 0.08;
export const HEAVY_RAIN_IN_HR = 0.3;
/** Below this the event is over (same rule as the Nowcast screen's end). */
export const RAIN_END_IN_HR = 0.05;
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
}

export interface LastRainAlert {
  key: string;
  startAt: number;
  firedAt: number;
}

export type Sample = { at: number; inHr: number };

function samples(points: RadarNowcastPoint[]): Sample[] {
  return points
    .map((point) => ({ at: new Date(point.timestamp).getTime(), inHr: point.precipitation_mm_h / 25.4 }))
    .filter((s) => Number.isFinite(s.at) && Number.isFinite(s.inHr))
    .sort((a, b) => a.at - b.at);
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
  const series = samples(nowcast.points);
  if (series.length < 2) return null;
  const horizonEnd = series[series.length - 1].at;
  if (horizonEnd <= now) return null;

  const current = intensityAt(series, now);
  if (current == null || current >= RAIN_START_IN_HR) return null; // raining already, or unknown

  let startAt: number | null = null;
  for (let t = now; t <= horizonEnd; t += 60_000) {
    const v = intensityAt(series, t);
    if (v != null && v >= RAIN_START_IN_HR) {
      startAt = t;
      break;
    }
  }
  if (startAt == null) return null;

  let endAt: number | null = null;
  let peak = 0;
  for (let t = startAt; t <= horizonEnd; t += 60_000) {
    const v = intensityAt(series, t) ?? 0;
    peak = Math.max(peak, v);
    if (v < RAIN_END_IN_HR) {
      endAt = t;
      break;
    }
  }

  const heavy = peak >= HEAVY_RAIN_IN_HR;
  const startInMin = Math.max(1, Math.round((startAt - now) / 60_000));
  const durationMin = endAt == null ? null : Math.max(1, Math.round((endAt - startAt) / 60_000));
  const lead = Math.max(1, prefs.leadMinutes) * 60_000;
  const fireAt = Math.max(now + 1_000, startAt - lead);
  const kind = heavy ? "Heavy rain" : "Rain";
  const where = place.trim() || "your location";
  const lasting = durationMin == null ? "" : `, lasting about ${durationMin} min`;
  return {
    key: `${nowcast.issued_at ?? "nowcast"}|${Math.round(startAt / SAME_EVENT_WINDOW_MS)}`,
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
