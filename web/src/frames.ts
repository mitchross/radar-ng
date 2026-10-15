/**
 * Manifest → timeline frames, and the playback sequence per zoom. Ported from
 * the app (frontend/src/hooks/useManifest.ts, lib/playbackSequence.ts) so the
 * web and app loops visit the same frames.
 */
import type { PackedMeta } from "./packed/format";

export interface ManifestFrame {
  timestamp: string;
  path: string;
  max_zoom?: number;
}

export interface Manifest {
  layers: Record<string, { frames?: ManifestFrame[]; timestamps?: string[]; packed?: PackedMeta }>;
}

export type FrameSource = "radar" | "nowcast" | "radar-hrrr" | string;

export interface Frame {
  /** Epoch seconds. */
  time: number;
  timestamp: string;
  path: string;
  /** The tile subtree the frame lives in. */
  source: FrameSource;
  maxZoom: number;
}

const DEFAULT_MAX_ZOOM: Record<string, number> = { radar: 7, "radar-composite": 7, nowcast: 6, "radar-hrrr": 6 };

function layerFrames(manifest: Manifest, key: string): Frame[] {
  const entry = manifest.layers[key];
  if (!entry) return [];
  const frames: ManifestFrame[] = entry.frames ?? (entry.timestamps ?? []).map((timestamp) => ({ timestamp, path: timestamp }));
  return frames.map((f) => ({
    time: Math.floor(Date.parse(f.timestamp) / 1000),
    timestamp: f.timestamp,
    path: f.path,
    source: key,
    maxZoom: f.max_zoom ?? DEFAULT_MAX_ZOOM[key] ?? 6,
  }));
}

/**
 * Radar merges observed MRMS, the 0–60 min nowcast and HRRR beyond; other layers are their own series.
 * `observed` picks the MRMS product for the past frames (base or composite reflectivity).
 */
export function buildFrames(manifest: Manifest, layer: string, nowSec: number, observed = "radar"): Frame[] {
  if (layer !== "radar") return layerFrames(manifest, layer);
  const source = manifest.layers[observed] ? observed : "radar";
  const past = layerFrames(manifest, source).filter((f) => f.time <= nowSec);
  const nowcast = layerFrames(manifest, "nowcast").filter((f) => f.time > nowSec && f.time <= nowSec + 3600);
  const hrrr = layerFrames(manifest, "radar-hrrr").filter((f) => f.time > nowSec + 3600);
  const seen = new Set<number>();
  return [...past, ...nowcast, ...hrrr]
    .sort((a, b) => a.time - b.time)
    .filter((f) => (seen.has(f.time) ? false : (seen.add(f.time), true)));
}

/**
 * True when two frame lists render the same timeline. The manifest poll is
 * usually unchanged, so it can skip the rebuild. Mirrors the app's sameFrames.
 */
export function sameFrames(a: readonly Frame[], b: readonly Frame[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i].time !== b[i].time || a[i].path !== b[i].path || a[i].source !== b[i].source || a[i].maxZoom !== b[i].maxZoom) return false;
  }
  return true;
}

export type Zoom = "1h" | "48h";
const HOUR = 3600;
const PAST_STEP_48H = 15 * 60;

/** Frame indices playback visits: 1h = everything within an hour of now; 48h = past thinned to 15 min + all future. */
export function playbackSequence(frames: readonly Frame[], zoom: Zoom, nowSec: number): number[] {
  if (frames.length === 0) return [];
  if (zoom === "1h") {
    const seq = frames.flatMap((f, i) => (f.time >= nowSec - HOUR && f.time <= nowSec + HOUR ? [i] : []));
    return seq.length > 0 ? seq : [nearestIndex(frames, nowSec)];
  }
  const past: number[] = [];
  let lastKept = Infinity;
  for (let i = frames.length - 1; i >= 0; i--) {
    const t = frames[i].time;
    if (t > nowSec) continue;
    if (lastKept - t >= PAST_STEP_48H) {
      past.push(i);
      lastKept = t;
    }
  }
  past.reverse();
  return [...past, ...frames.flatMap((f, i) => (f.time > nowSec ? [i] : []))];
}

export function nearestIndex(frames: readonly Frame[], sec: number): number {
  let best = 0;
  for (let i = 1; i < frames.length; i++) {
    if (Math.abs(frames[i].time - sec) < Math.abs(frames[best].time - sec)) best = i;
  }
  return best;
}

/** The latest observed frame at or before now, else the frame nearest now. */
export function nowIndex(frames: readonly Frame[], nowSec: number): number {
  for (let i = frames.length - 1; i >= 0; i--) {
    if (frames[i].source !== "nowcast" && frames[i].source !== "radar-hrrr" && frames[i].time <= nowSec) return i;
  }
  return nearestIndex(frames, nowSec);
}

/**
 * The time the playhead keeps after a manifest refresh, or undefined to follow
 * "now". Hold while playing or scrubbed; only a playhead parked on the live
 * frame follows the newest observation.
 */
export function refreshKeepTime(current: Frame | undefined, live: Frame | undefined, playing: boolean): number | undefined {
  if (!current) return undefined;
  if (!playing && live && current.time === live.time) return undefined;
  return current.time;
}

/** "Now", "+45m", "−4h". */
export function offsetLabel(frameSec: number, nowSec: number): string {
  const min = Math.round((frameSec - nowSec) / 60);
  if (Math.abs(min) <= 5) return "Now";
  const sign = min > 0 ? "+" : "−";
  const abs = Math.abs(min);
  return abs < 90 ? `${sign}${abs}m` : `${sign}${Math.round(abs / 60)}h`;
}
