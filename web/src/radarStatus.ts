/**
 * Radar freshness for the timeline chip and the notice above it. Ported from
 * the app (frontend/src/lib/radarStatus.ts, components/map/RadarStatusNotice.tsx)
 * so both say the same thing about the same radar.
 */
import type { Manifest } from "./frames";

const LIVE_MAX_AGE_SECONDS = 10 * 60;

export type RadarStatusTone = "live" | "stale" | "offline" | "unavailable";

export interface RadarStatus {
  label: string;
  accessibilityLabel: string;
  tone: RadarStatusTone;
}

interface RadarStatusInput {
  frameTimeSeconds: number | null;
  refreshFailed: boolean;
  loading: boolean;
  nowMilliseconds?: number;
}

function ageLabel(ageMinutes: number): { badge: string; spoken: string } {
  if (ageMinutes < 60) {
    return { badge: `${ageMinutes}M`, spoken: `${ageMinutes} minute${ageMinutes === 1 ? "" : "s"}` };
  }
  const ageHours = Math.floor(ageMinutes / 60);
  if (ageHours < 24) {
    return { badge: `${ageHours}H`, spoken: `${ageHours} hour${ageHours === 1 ? "" : "s"}` };
  }
  const ageDays = Math.floor(ageHours / 24);
  return { badge: `${ageDays}D`, spoken: `${ageDays} day${ageDays === 1 ? "" : "s"}` };
}

/** Derive a truthful status from the newest observation, not manifest age. */
export function radarStatus({ frameTimeSeconds, refreshFailed, loading, nowMilliseconds = Date.now() }: RadarStatusInput): RadarStatus {
  if (frameTimeSeconds === null) {
    return loading && !refreshFailed
      ? { label: "LOADING", accessibilityLabel: "Radar loading", tone: "unavailable" }
      : { label: "UNAVAILABLE", accessibilityLabel: "Radar unavailable", tone: "unavailable" };
  }
  if (refreshFailed) {
    return { label: "OFFLINE", accessibilityLabel: "Radar offline. Showing the last saved observation", tone: "offline" };
  }
  const ageSeconds = Math.max(0, Math.floor(nowMilliseconds / 1000) - frameTimeSeconds);
  if (ageSeconds <= LIVE_MAX_AGE_SECONDS) {
    return { label: "LIVE", accessibilityLabel: "Live radar available", tone: "live" };
  }
  const age = ageLabel(Math.max(1, Math.floor(ageSeconds / 60)));
  return { label: `UPDATED ${age.badge} AGO`, accessibilityLabel: `Radar updated ${age.spoken} ago`, tone: "stale" };
}

/** What to say when the radar is loading, offline or unavailable; null for live and stale. */
export function radarNoticeCopy(status: RadarStatus, hasFrames: boolean): { title: string; message: string; retry: boolean } | null {
  if (status.tone === "unavailable") {
    if (status.label === "LOADING" && !hasFrames) {
      return { title: "Loading radar", message: "Fetching the latest frames from your server.", retry: false };
    }
    return { title: "Radar unavailable", message: "The radar server is unreachable. Check the connection and try again.", retry: true };
  }
  if (status.tone === "offline") {
    return { title: "Radar offline", message: "Showing the last saved observation until the server answers.", retry: true };
  }
  return null;
}

/**
 * Time of the newest observed MRMS frame at or before now, whatever layer is
 * shown, so a forecast layer still reports the server's observation health.
 */
export function observedFrameTime(manifest: Manifest | null, nowSec: number): number | null {
  const layer = manifest?.layers.radar ?? manifest?.layers["radar-composite"];
  const stamps = layer?.frames?.map((f) => f.timestamp) ?? layer?.timestamps ?? [];
  let best: number | null = null;
  for (const stamp of stamps) {
    const t = Math.floor(Date.parse(stamp) / 1000);
    if (t <= nowSec && (best === null || t > best)) best = t;
  }
  return best;
}
