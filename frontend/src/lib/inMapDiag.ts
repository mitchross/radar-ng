/**
 * Diagnostics and the blank-radar safety net of the in-map engine. Pure and module-level: the engine
 * writes `inMapStats`, Settings shows `formatInMapSummary`, telemetry sends `inMapDiagAttributes`,
 * and `decideInMapFallback` says when to draw the Classic fast overlay instead. No Skia, no file system.
 */
import type { TelemetryAttributes } from "./telemetryPrivacy";

/** Where the engine failed: the stage tells which part of the pipeline to look at. */
export type InMapStage = "surface" | "atlas-load" | "color" | "encode" | "write" | "copy" | "verify";

export type InMapSurfaceKind = "gpu" | "cpu" | "none" | "unknown";

export interface InMapStats {
  surface: InMapSurfaceKind;
  /** Frames (gens) committed since the process started. */
  framesCommitted: number;
  tilesWritten: number;
  bytesWritten: number;
  lastPngBytes: number | null;
  /** Whether the last encoded PNG starts with the PNG signature. */
  lastPngSignature: boolean | null;
  lastError: { stage: InMapStage; message: string } | null;
  errorCount: number;
  /** URI of the first tile of the first committed gen, and whether it was on disk after the write. */
  firstTileUri: string | null;
  firstTileExists: boolean | null;
  /** Result of the latest verify step; false = a committed gen had no file. */
  verifyOk: boolean | null;
  mapZoom: number | null;
  /** Zoom of the tiles last written. */
  writtenZoom: number | null;
  urlTemplate: string | null;
  /** Why the Classic fast overlay is drawing instead, or null while in-map layers draw. */
  fallback: string | null;
}

const initial = (): InMapStats => ({
  surface: "unknown",
  framesCommitted: 0,
  tilesWritten: 0,
  bytesWritten: 0,
  lastPngBytes: null,
  lastPngSignature: null,
  lastError: null,
  errorCount: 0,
  firstTileUri: null,
  firstTileExists: null,
  verifyOk: null,
  mapZoom: null,
  writtenZoom: null,
  urlTemplate: null,
  fallback: null,
});

/** The live stats object; mutate through the helpers below so listeners hear about it. */
export const inMapStats: InMapStats = initial();

const listeners = new Set<() => void>();
let version = 0;
let notifyTimer: ReturnType<typeof setTimeout> | null = null;

function changed() {
  version++;
  if (listeners.size === 0 || notifyTimer) return;
  // Tiles are written many times a second; the Settings line does not need to repaint that often.
  notifyTimer = setTimeout(() => {
    notifyTimer = null;
    listeners.forEach((l) => l());
  }, 400);
}

export const subscribeInMapStats = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

/** Changes whenever a stat does; the snapshot for useSyncExternalStore. */
export const inMapStatsVersion = () => version;

export function patchInMapStats(patch: Partial<InMapStats>) {
  Object.assign(inMapStats, patch);
  changed();
}

/** Back to the initial values (tests). */
export function resetInMapStats() {
  Object.assign(inMapStats, initial());
  version++;
}

const MESSAGE_MAX = 120;

/** A short single-line message with any URL or path replaced, so nothing identifying leaves the phone. */
export function errorMessage(err: unknown): string {
  const raw = err instanceof Error ? err.message : typeof err === "string" ? err : "unknown";
  return raw
    .replace(/\S*:\/\/\S*/g, "<uri>")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, MESSAGE_MAX);
}

/** Record a failure at a stage; the caller carries on exactly as it did before. */
export function recordInMapError(stage: InMapStage, err: unknown) {
  inMapStats.lastError = { stage, message: errorMessage(err) };
  inMapStats.errorCount++;
  changed();
  onNewErrorStage?.(stage);
}

let onNewErrorStage: ((stage: InMapStage) => void) | undefined;
/** Hook for the telemetry reporter: called on every recorded error. */
export function setInMapErrorListener(fn: ((stage: InMapStage) => void) | undefined) {
  onNewErrorStage = fn;
}

export const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47] as const;

export function hasPngSignature(bytes: Uint8Array): boolean {
  return bytes.length >= PNG_SIGNATURE.length && PNG_SIGNATURE.every((b, i) => bytes[i] === b);
}

/** The path under the cache root without the tile's z/x/y, e.g. `radar-inmap/radar/bucket.v1.s0.ab12/3`. */
export function genSuffix(uri: string | null): string | null {
  if (!uri) return null;
  const at = uri.indexOf("radar-inmap/");
  if (at < 0) return null;
  const parts = uri.slice(at).split("/");
  return parts.slice(0, -3).join("/") || null;
}

/** How long the radar may show with a cover before In-map without a committed frame gives up. */
export const IN_MAP_BLANK_MS = 8_000;

export interface FallbackInput {
  /** `canRenderInMap()`: an offscreen Skia surface exists. */
  canRender: boolean;
  /** The radar is showing and has tiles in view to color. */
  hasCover: boolean;
  /** Milliseconds since the radar showed with a non-empty cover (0 when it does not). */
  waitedMs: number;
  /** Gens committed and verified, still in the engine's disk index. */
  committedFrames: number;
  /** The latest verify step found no file. */
  verifyFailed: boolean;
}

/**
 * Why to draw the Classic fast overlay instead of the in-map layers, or null to draw in-map. Re-run
 * as the inputs change: the engine keeps trying in the background, and the first verified frame
 * clears the reason.
 */
export function decideInMapFallback(input: FallbackInput): string | null {
  if (!input.canRender) return "no offscreen surface";
  if (input.verifyFailed) return "verify: no file on disk";
  if (input.hasCover && input.committedFrames === 0 && input.waitedMs >= IN_MAP_BLANK_MS) {
    return `no frame in ${Math.round(IN_MAP_BLANK_MS / 1000)}s`;
  }
  return null;
}

const SURFACE_LABEL: Record<InMapSurfaceKind, string> = { gpu: "GPU", cpu: "CPU", none: "no surface", unknown: "surface ?" };

const formatBytes = (n: number) => (n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.round(n / 1024)} KB`);

/** Two short lines for Settings: what is happening, then what went wrong or what was last seen. */
export function formatInMapSummary(s: InMapStats): string[] {
  const first = [
    `In-map: ${s.framesCommitted} frames`,
    `${s.tilesWritten} tiles`,
    formatBytes(s.bytesWritten),
    SURFACE_LABEL[s.surface],
  ].join(" · ");
  const second: string[] = [];
  if (s.fallback) second.push(`fallback: ${s.fallback}`);
  second.push(s.lastError ? `last error: ${s.lastError.stage} ${s.lastError.message}` : "no errors");
  if (s.mapZoom != null) second.push(`zoom ${s.mapZoom.toFixed(1)}${s.writtenZoom != null ? ` / z${s.writtenZoom}` : ""}`);
  if (s.lastPngBytes != null) second.push(`png ${s.lastPngBytes} B ${s.lastPngSignature ? "ok" : "bad"}`);
  if (s.firstTileExists != null) second.push(`file ${s.firstTileExists ? "ok" : "missing"}`);
  return [first, second.join(" · ")];
}

/** Telemetry attributes for the `inmap_diag` event: counts, zoom and errors only, no coordinates. */
export function inMapDiagAttributes(s: InMapStats, reason: string): TelemetryAttributes {
  const out: TelemetryAttributes = {
    "inmap.reason": reason,
    "inmap.surface": s.surface,
    "inmap.frames": s.framesCommitted,
    "inmap.tiles": s.tilesWritten,
    "inmap.bytes": s.bytesWritten,
    "inmap.errors": s.errorCount,
  };
  if (s.lastPngBytes != null) out["inmap.png_bytes"] = s.lastPngBytes;
  if (s.lastPngSignature != null) out["inmap.png_ok"] = s.lastPngSignature;
  if (s.lastError) {
    out["inmap.err_stage"] = s.lastError.stage;
    out["inmap.err_msg"] = s.lastError.message;
  }
  if (s.firstTileExists != null) out["inmap.first_tile_exists"] = s.firstTileExists;
  if (s.verifyOk != null) out["inmap.verify_ok"] = s.verifyOk;
  if (s.mapZoom != null) out["inmap.zoom"] = Math.round(s.mapZoom * 10) / 10;
  if (s.writtenZoom != null) out["inmap.written_z"] = s.writtenZoom;
  const suffix = genSuffix(s.firstTileUri);
  if (suffix) out["inmap.tile_dir"] = suffix;
  if (s.fallback) out["inmap.fallback"] = s.fallback;
  return out;
}

export const DIAG_EVENT_INTERVAL_MS = 60_000;

/**
 * Whether an `inmap_diag` event is due: at most one a minute, plus one for every error stage the
 * first time it is seen. `seenStages` is the caller's set of stages already sent (mutated here).
 */
export function diagEventDue(lastSentAt: number | null, now: number, newStage: InMapStage | null, seenStages: Set<InMapStage>): boolean {
  if (newStage && !seenStages.has(newStage)) {
    seenStages.add(newStage);
    return true;
  }
  return lastSentAt == null || now - lastSentAt >= DIAG_EVENT_INTERVAL_MS;
}
