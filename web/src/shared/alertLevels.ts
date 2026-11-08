export type AlertLevel = "chime" | "warning" | "critical";
export type AlertPreference = "warnings" | "watches" | "everything";
export const ALERT_PREFERENCES: readonly { value: AlertPreference; label: string }[] = [
  { value: "warnings", label: "Warnings only" },
  { value: "watches", label: "Warnings + watches" },
  { value: "everything", label: "Everything" },
];

export interface AlertProperties {
  id?: string;
  event: string;
  severity: string;
  headline?: string | null;
  expires?: string | null;
  ends?: string | null;
  sent?: string;
  status?: string;
  messageType?: string;
  parameters?: Record<string, string[]>;
  references?: { identifier: string }[];
}
export interface AlertFeature {
  id?: string;
  properties: AlertProperties;
  geometry?: { type: string; coordinates: unknown } | null;
}
export type AlertSeen = Record<string, number>;
export type AlertBounds = [number, number, number, number];

export function parseAlertPreference(value: string): AlertPreference {
  return value === "watches" || value === "everything" ? value : "warnings";
}

/** NWS CAP event names override severity: a Severe watch is still a chime. */
export function alertLevel(p: AlertProperties): AlertLevel {
  if (p.event === "Tornado Warning" || /Emergency\b/i.test(p.event) || /\bEmergency\b/i.test(p.headline ?? "") ||
    (p.event === "Severe Thunderstorm Warning" && p.parameters?.thunderstormDamageThreat?.some(v => v.toUpperCase() === "DESTRUCTIVE"))) return "critical";
  if (/\bWarning\b/i.test(p.event)) return "warning";
  if (/\b(Watch|Advisory|Statement|Outlook)\b/i.test(p.event)) return "chime";
  return p.severity === "Extreme" || p.severity === "Severe" ? "warning" : "chime";
}

export function parseAlertSeen(raw: string): AlertSeen {
  try {
    const value: unknown = JSON.parse(raw);
    if (value && typeof value === "object" && !Array.isArray(value)) {
      return Object.fromEntries(Object.entries(value).filter(([, expiry]) => typeof expiry === "number" && Number.isFinite(expiry)));
    }
  } catch { /* no saved history */ }
  return {};
}

function canonicalId(id: string): string {
  return id.replace(/^https:\/\/api\.weather\.gov\/alerts\//, "");
}

function alertKeys(a: AlertFeature, now: number): string[] {
  const p = a.properties;
  const ids = [a.id, p.id, ...p.references?.map(r => r.identifier) ?? []]
    .filter((id): id is string => Boolean(id)).map(id => `id:${canonicalId(id)}`);
  for (const vtec of p.parameters?.VTEC ?? []) {
    // Ignore NEW/CON/EXT/etc.; office, phenomenon, significance, event number and year identify the event.
    for (const match of vtec.matchAll(/\/[OTEX]\.\w{3}\.([A-Z0-9]{4})\.([A-Z]{2})\.([WAYFSNO])\.(\d{4})\.(\d{6})T\d{4}Z-/g)) {
      const year = match[5] === "000000" ? new Date(p.sent ?? now).getUTCFullYear() : 2000 + Number(match[5].slice(0, 2));
      ids.push(`vtec:${match[1]}:${match[2]}:${match[3]}:${match[4]}:${year}`);
    }
  }
  return ids;
}

/** Record every update, filtered alert and overflow before returning at most five interruptions. */
export function selectAlertNotifications(alerts: readonly AlertFeature[], previous: AlertSeen, preference: AlertPreference, now = Date.now()): { fresh: AlertFeature[]; seen: AlertSeen } {
  const seen = Object.fromEntries(Object.entries(previous).filter(([, expiry]) => expiry > now));
  const rank = { chime: 0, warning: 1, critical: 2 };
  const severity = { Extreme: 4, Severe: 3, Moderate: 2, Minor: 1, Unknown: 0 };
  const sorted = [...alerts].sort((a, b) => rank[alertLevel(b.properties)] - rank[alertLevel(a.properties)] ||
    (severity[b.properties.severity as keyof typeof severity] ?? 0) - (severity[a.properties.severity as keyof typeof severity] ?? 0));
  const fresh: AlertFeature[] = [];
  for (const a of sorted) {
    const p = a.properties;
    const keys = alertKeys(a, now);
    if (keys.length === 0) continue;
    const known = keys.some(k => Object.hasOwn(seen, k));
    const end = Date.parse(p.ends ?? p.expires ?? "");
    // Long-lived alerts retain their identities past expiration; short alerts retain references for 30 days.
    const retainUntil = Math.max(now + 30 * 86400_000, Number.isFinite(end) ? end + 86400_000 : 0);
    for (const key of keys) seen[key] = retainUntil;
    if (known || (p.status && p.status !== "Actual") || ["Cancel", "Ack", "Error"].includes(p.messageType ?? "") || (Number.isFinite(end) && end <= now)) continue;
    const allowed = alertLevel(p) !== "chime" || preference === "everything" || (preference === "watches" && /\bWatch\b/i.test(p.event));
    if (allowed && fresh.length < 5) fresh.push(a);
  }
  return { fresh, seen };
}

export function polygonAlertBounds(geometry: AlertFeature["geometry"]): AlertBounds | null {
  if (!geometry || !["Polygon", "MultiPolygon"].includes(geometry.type)) return null;
  const points: number[][] = [];
  const visit = (value: unknown): void => {
    if (!Array.isArray(value)) return;
    if (value.length >= 2 && typeof value[0] === "number" && typeof value[1] === "number") {
      if (Number.isFinite(value[0]) && Number.isFinite(value[1]) && Math.abs(value[0]) <= 180 && Math.abs(value[1]) <= 90) points.push(value);
    } else value.forEach(visit);
  };
  visit(geometry.coordinates);
  if (points.length < 3) return null;
  return [Math.min(...points.map(p => p[0])), Math.min(...points.map(p => p[1])), Math.max(...points.map(p => p[0])), Math.max(...points.map(p => p[1]))];
}

export function parseAlertBounds(value: unknown): AlertBounds | null {
  if (!Array.isArray(value) || value.length !== 4 || !value.every(v => typeof v === "number" && Number.isFinite(v))) return null;
  const [west, south, east, north] = value;
  return west >= -180 && east <= 180 && south >= -90 && north <= 90 && west <= east && south <= north
    ? [west, south, east, north] : null;
}

export const TEST_ALERT = { title: "TEST — Radar NG warning", body: "This is a test alert. No weather emergency is being reported." };
