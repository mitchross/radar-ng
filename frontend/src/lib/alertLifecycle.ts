import type { NWSAlert, NWSAlertCollection } from "../types/weather";

const ISO_DATE_TIME =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?(?:Z|[+-](\d{2}):(\d{2}))$/;
const MAX_TIMER_DELAY_MS = 2_147_483_647;

export interface AlertWindow {
  startsAt: number;
  endsAt: number;
}

export interface AlertCollectionSnapshot {
  collection: NWSAlertCollection;
  invalidCount: number;
  nextTransitionAt: number | null;
}

export type AlertFreshnessKind = "current" | "stale" | "offline" | "unavailable";

export interface AlertFreshnessStatus {
  kind: AlertFreshnessKind;
  label: string;
  accessibilityLabel: string;
}

/** Parse only the complete ISO-8601 timestamps promised by api.weather.gov. */
export function parseNwsTimestamp(value: unknown): number | null {
  if (typeof value !== "string") return null;
  const match = ISO_DATE_TIME.exec(value);
  if (!match) return null;

  const [, yearText, monthText, dayText, hourText, minuteText, secondText, offsetHourText, offsetMinuteText] = match;
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const hour = Number(hourText);
  const minute = Number(minuteText);
  const second = Number(secondText);
  const offsetHour = Number(offsetHourText ?? 0);
  const offsetMinute = Number(offsetMinuteText ?? 0);
  if (
    year < 2000 ||
    year > 2200 ||
    month < 1 ||
    month > 12 ||
    day < 1 ||
    hour > 23 ||
    minute > 59 ||
    second > 59 ||
    offsetHour > 14 ||
    offsetMinute > 59 ||
    (offsetHour === 14 && offsetMinute !== 0)
  ) {
    return null;
  }

  const daysInMonth = new Date(Date.UTC(year, month, 0)).getUTCDate();
  if (day > daysInMonth) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * CAP `effective` controls visibility; `onset` is only its defensive fallback.
 * `expires` bounds message freshness, while an earlier `ends` bounds the event.
 */
export function getAlertWindow(alert: NWSAlert): AlertWindow | null {
  const properties = alert.properties as NWSAlert["properties"] & Record<string, unknown>;
  const effective = properties.effective;
  const startsAt = effective === null || effective === undefined
    ? parseNwsTimestamp(properties.onset)
    : parseNwsTimestamp(effective);
  const expiresAt = parseNwsTimestamp(properties.expires);
  if (startsAt === null || expiresAt === null) return null;

  const rawEnds = properties.ends;
  const eventEndsAt = rawEnds === null || rawEnds === undefined
    ? null
    : parseNwsTimestamp(rawEnds);
  if (rawEnds !== null && rawEnds !== undefined && eventEndsAt === null) return null;

  const endsAt = eventEndsAt === null ? expiresAt : Math.min(expiresAt, eventEndsAt);
  return endsAt > startsAt ? { startsAt, endsAt } : null;
}

export function getAlertEndTime(alert: NWSAlert): number | null {
  return getAlertWindow(alert)?.endsAt ?? null;
}

/** Derive visible alerts from cached source data at a specific wall-clock time. */
export function getAlertCollectionSnapshot(
  collection: NWSAlertCollection,
  nowMilliseconds = Date.now(),
): AlertCollectionSnapshot {
  const features: NWSAlert[] = [];
  let invalidCount = 0;
  let nextTransitionAt: number | null = null;

  for (const alert of collection.features) {
    const window = getAlertWindow(alert);
    if (!window) {
      invalidCount += 1;
      continue;
    }

    if (nowMilliseconds < window.startsAt) {
      nextTransitionAt = earliest(nextTransitionAt, window.startsAt);
      continue;
    }
    if (nowMilliseconds >= window.endsAt) continue;

    features.push(alert);
    nextTransitionAt = earliest(nextTransitionAt, window.endsAt);
  }

  return {
    collection: { ...collection, features },
    invalidCount,
    nextTransitionAt,
  };
}

function earliest(current: number | null, candidate: number): number {
  return current === null ? candidate : Math.min(current, candidate);
}

/** Schedule the local re-evaluation that removes an alert at its next boundary. */
export function scheduleAlertTransition(
  transitionAt: number | null,
  onTransition: () => void,
  nowMilliseconds = Date.now(),
): () => void {
  if (transitionAt === null) return () => undefined;
  const delay = Math.min(
    MAX_TIMER_DELAY_MS,
    Math.max(0, transitionAt - nowMilliseconds),
  );
  const timer = setTimeout(onTransition, delay);
  return () => clearTimeout(timer);
}

export function getAlertFreshnessStatus(input: {
  hasCachedData: boolean;
  activeCount: number;
  invalidCount: number;
  isOnline: boolean;
  refreshFailed: boolean;
  isPending: boolean;
}): AlertFreshnessStatus {
  if (!input.isOnline) {
    return {
      kind: "offline",
      label: "OFFLINE",
      accessibilityLabel: input.hasCachedData && input.activeCount > 0
        ? "Weather alerts offline. Showing still-active alerts from the last successful check."
        : "Weather alert status unavailable while offline.",
    };
  }

  if (input.refreshFailed) {
    return input.hasCachedData
      ? {
          kind: "stale",
          label: "STALE",
          accessibilityLabel: input.activeCount > 0
            ? "Weather alert update failed. Showing still-active alerts from the last successful check."
            : "Weather alert status unavailable because the last update failed.",
        }
      : {
          kind: "unavailable",
          label: "UNAVAILABLE",
          accessibilityLabel: "Weather alerts unavailable. The National Weather Service could not be reached.",
        };
  }

  if (input.invalidCount > 0) {
    return {
      kind: "unavailable",
      label: input.activeCount > 0 ? "PARTIAL DATA" : "UNAVAILABLE",
      accessibilityLabel: input.activeCount > 0
        ? "Some National Weather Service alerts could not be verified."
        : "Weather alert status unavailable because alert times could not be verified.",
    };
  }

  if (!input.hasCachedData && !input.isPending) {
    return {
      kind: "unavailable",
      label: "UNAVAILABLE",
      accessibilityLabel: "Weather alerts unavailable.",
    };
  }

  return {
    kind: "current",
    label: "CURRENT",
    accessibilityLabel: "Weather alert status current.",
  };
}
