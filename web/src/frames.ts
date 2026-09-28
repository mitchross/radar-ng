/**
 * Manifest → timeline frames, and the playback sequence per zoom. Ported from
 * the app (frontend/src/hooks/useManifest.ts, lib/playbackSequence.ts) so the
 * web and app loops visit the same frames.
 */
export interface ManifestFrame {
  timestamp: string;
  path: string;
  max_zoom?: number;
}

export interface Manifest {
  layers: Record<string, { frames?: ManifestFrame[]; timestamps?: string[] }>;
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

/** Radar merges observed MRMS, the 0–60 min nowcast and HRRR beyond; other layers are their own series. */
export function buildFrames(manifest: Manifest, layer: string, nowSec: number): Frame[] {
  if (layer !== "radar") return layerFrames(manifest, layer);
  const past = layerFrames(manifest, "radar").filter((f) => f.time <= nowSec);
  const nowcast = layerFrames(manifest, "nowcast").filter((f) => f.time > nowSec && f.time <= nowSec + 3600);
  const hrrr = layerFrames(manifest, "radar-hrrr").filter((f) => f.time > nowSec + 3600);
  const seen = new Set<number>();
  return [...past, ...nowcast, ...hrrr]
    .sort((a, b) => a.time - b.time)
    .filter((f) => (seen.has(f.time) ? false : (seen.add(f.time), true)));
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

/** "Now", "+45m", "−4h". */
export function offsetLabel(frameSec: number, nowSec: number): string {
  const min = Math.round((frameSec - nowSec) / 60);
  if (Math.abs(min) <= 5) return "Now";
  const sign = min > 0 ? "+" : "−";
  const abs = Math.abs(min);
  return abs < 90 ? `${sign}${abs}m` : `${sign}${Math.round(abs / 60)}h`;
}
