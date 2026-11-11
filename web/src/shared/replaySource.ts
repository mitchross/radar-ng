/** Replay source context and timeline rules shared by the web radar and the app. */
import { replayWarningsAt, type ReplayWarning, type ReplayWarningHistory } from "./replayWarnings";

export type SourceContext = { kind: "live" } | { kind: "replay"; caseId: string; revision: string };

export const LIVE_SOURCE: SourceContext = { kind: "live" };

export const isReplaySource = (s: SourceContext): s is Extract<SourceContext, { kind: "replay" }> => s.kind === "replay";

/** Cache-key part: frames, tiles and manifests of one source never share a key with another. */
export const sourceKey = (s: SourceContext): string => (s.kind === "live" ? "live" : `replay:${s.caseId}:${s.revision}`);

export const sameSource = (a: SourceContext, b: SourceContext): boolean => sourceKey(a) === sourceKey(b);

/** Layers a case can show; the live manifest's other layers never appear in replay. */
export const REPLAY_LAYERS = ["radar", "radar-composite", "hail-size", "rotation-track", "echo-top", "precip-24h"] as const;

/** Replay tiles exist only in the classic palette. */
export const REPLAY_PALETTE = "classic";

/** Stale-source guard: switching source aborts in-flight requests and voids late results. */
export class SourceGuard {
  private controller = new AbortController();

  /** Abort everything started so far. Call on every source switch. */
  cancel(): void {
    this.controller.abort();
    this.controller = new AbortController();
  }

  /** A request ticket: its signal aborts on `cancel()`, and `current()` is false after it. */
  ticket(): { signal: AbortSignal; current: () => boolean } {
    const { signal } = this.controller;
    return { signal, current: () => !signal.aborted };
  }
}

export interface ReplayManifestFrame { timestamp: string; path: string; max_zoom?: number }
export interface ReplayManifest {
  schema_version?: number;
  tile_url_template?: string;
  layers: Record<string, { frames?: ReplayManifestFrame[]; timestamps?: string[] }>;
}

export interface ReplayFrame {
  /** Master playhead (base reflectivity) in epoch seconds. */
  time: number;
  /** Valid time of the layer frame shown. */
  timestamp: string;
  path: string;
  /** The layer key whose tile tree the frame lives in. */
  source: string;
  maxZoom: number;
  /** No layer frame within the staleness cap: draw nothing and say so. */
  gap?: boolean;
}

/** Fast layers may lag the base scan by five minutes; hourly rainfall by 90. */
const STALE_CAP_S: Record<string, number> = { "precip-24h": 90 * 60 };
const DEFAULT_STALE_CAP_S = 5 * 60;
const DEFAULT_MAX_ZOOM = 7;

const framesOf = (manifest: ReplayManifest, key: string): ReplayManifestFrame[] => {
  const entry = manifest.layers[key];
  if (!entry) return [];
  return entry.frames ?? (entry.timestamps ?? []).map((timestamp) => ({ timestamp, path: timestamp }));
};

/**
 * One entry per base reflectivity frame, no forecast merging, age filter or thinning.
 * Each shows the layer's latest frame at or before that time, or a gap beyond the cap.
 */
export function replayFrames(manifest: ReplayManifest, layer: string): ReplayFrame[] {
  const master = framesOf(manifest, "radar").map((f) => ({ f, t: Math.floor(Date.parse(f.timestamp) / 1000) }));
  const own = framesOf(manifest, layer).map((f) => ({ f, t: Math.floor(Date.parse(f.timestamp) / 1000) }));
  if (own.length === 0) return [];
  const cap = STALE_CAP_S[layer] ?? DEFAULT_STALE_CAP_S;
  let at = -1;
  return master.map(({ t }) => {
    while (at + 1 < own.length && own[at + 1].t <= t) at++;
    const pick = own[Math.max(at, 0)];
    return {
      time: t,
      timestamp: pick.f.timestamp,
      path: pick.f.path,
      source: layer,
      maxZoom: pick.f.max_zoom ?? DEFAULT_MAX_ZOOM,
      ...(at < 0 || t - pick.t > cap ? { gap: true } : {}),
    };
  });
}

/** Every index, in order: replay loops the whole case. */
export const fullCaseSequence = (count: number): number[] => Array.from({ length: Math.max(0, count) }, (_, i) => i);

/** Indices after `pos` that prefetch should warm, wrapping at the loop end; never more than `ahead`. */
export function lookaheadPositions(pos: number, length: number, ahead: number): number[] {
  if (length <= 0) return [];
  return Array.from({ length: Math.min(ahead, length - 1) + 1 }, (_, k) => (pos + k) % length);
}

/** Fill the layer, palette and path of a manifest tile template; z/x/y stay for the map. */
export function replayTileUrl(template: string, layer: string, palette: string, path: string): string {
  return template
    .replace("{layer}", encodeURIComponent(layer))
    .replace("{palette}", encodeURIComponent(palette))
    .replace("{path}", path.split("/").map(encodeURIComponent).join("/"));
}

/** The playhead's absolute date, time and zone in the case's zone, e.g. "Fri, Mar 24, 2023, 7:57 PM CDT". */
export function replayTimeLabel(sec: number, timeZone: string): string {
  return new Intl.DateTimeFormat("en-US", {
    timeZone, weekday: "short", year: "numeric", month: "short", day: "numeric",
    hour: "numeric", minute: "2-digit", timeZoneName: "short",
  }).format(new Date(sec * 1000));
}

/** Short axis label: clock time and zone only, "7:57 PM CDT". */
export function replayClockLabel(sec: number, timeZone: string): string {
  return new Intl.DateTimeFormat("en-US", { timeZone, hour: "numeric", minute: "2-digit", timeZoneName: "short" }).format(new Date(sec * 1000));
}

/** Second timeline line: a gap notice, or the valid time of a layer that lags the playhead. */
export function replayFrameNote(frame: ReplayFrame, timeZone: string): string {
  if (frame.gap) return "No data for this layer at this time";
  const valid = Math.floor(Date.parse(frame.timestamp) / 1000);
  if (frame.source === "precip-24h") return `Rolling 24-hour total, valid ${replayClockLabel(valid, timeZone)}`;
  return frame.time === valid ? "" : `Latest data from ${replayClockLabel(valid, timeZone)}`;
}

export interface ReplayWarningProps {
  id: string;
  event: string;
  areaDesc: string;
  ends: string;
  status: string;
  wfo: string;
  eventid: number;
  begins: string;
  emergency: boolean;
  pds: boolean;
}

export interface ReplayFeature { type: "Feature"; id: string; geometry: unknown; properties: ReplayWarningProps }
export interface ReplayFeatureCollection { type: "FeatureCollection"; features: ReplayFeature[] }

const eventName = (w: ReplayWarning) => (w.properties.event ? w.properties.event : w.properties.phenomena);

/** GeoJSON features in force at the playhead, shaped for the live warning layers. */
export function replayWarningCollection(history: ReplayWarningHistory | null, timeMs: number): ReplayFeatureCollection {
  const features = history ? replayWarningsAt(history, timeMs) : [];
  return {
    type: "FeatureCollection",
    features: features.map((w) => ({
      type: "Feature",
      id: w.id,
      geometry: w.geometry,
      properties: {
        id: `${w.properties.wfo}-${w.properties.phenomena}-${w.properties.eventid}`,
        event: eventName(w),
        areaDesc: `NWS ${w.properties.wfo} #${w.properties.eventid}`,
        ends: w.properties.utc_polygon_end,
        status: w.properties.status,
        wfo: w.properties.wfo,
        eventid: w.properties.eventid,
        begins: w.properties.utc_polygon_begin,
        emergency: w.properties.is_emergency,
        pds: w.properties.is_pds,
      },
    })),
  };
}

/** Stable key of the versions in force, so clients redraw only when the set changes. */
export const replayWarningKey = (fc: ReplayFeatureCollection): string => fc.features.map((f) => String(f.id)).join("|");

export interface ReplayWarningCopy { title: string; badge: string; lines: string[] }

/** Popup text for a warning version, in the case's zone. */
export function replayWarningCopy(p: Partial<ReplayWarningProps>, timeZone: string): ReplayWarningCopy {
  const ends = p.ends ? Math.floor(Date.parse(p.ends) / 1000) : null;
  const begins = p.begins ? Math.floor(Date.parse(p.begins) / 1000) : null;
  return {
    title: String(p.event ?? "Warning"),
    badge: p.emergency ? "Emergency" : p.pds ? "Particularly dangerous situation" : "",
    lines: [
      p.areaDesc ?? "",
      begins != null && ends != null ? `This update ${replayClockLabel(begins, timeZone)} to ${replayClockLabel(ends, timeZone)}` : "",
      p.status ? `Status ${p.status}` : "",
      "Historical warning, not a live alert",
    ].filter(Boolean),
  };
}
