import { REPLAY_LAYERS, replayFrames, type ReplayManifest } from "./replaySource";
import type { LayerType, RadarFrame } from "../types/weather";

/** Replay frames in the app's frame model: one per base scan, no merging or thinning. */
export function replayRadarFrames(manifest: ReplayManifest, layer: string): RadarFrame[] {
  return replayFrames(manifest, layer).map((f) => ({
    time: f.time,
    timestamp: f.timestamp,
    path: f.path,
    source: f.source as RadarFrame["source"],
    maxZoom: f.maxZoom,
    ...(f.gap ? { gap: true } : {}),
  }));
}

export const isReplayLayer = (layer: string): boolean => (REPLAY_LAYERS as readonly string[]).includes(layer);

/** The layer to show on entering a case: keep the current one if the case has it, else radar. */
export function layerForCase(current: LayerType, caseLayers: readonly string[]): LayerType {
  return isReplayLayer(current) && caseLayers.includes(current) ? current : "radar";
}

/** Live frames show only once the manifest was fetched after the last return to live. */
export function liveFramesReady(dataUpdatedAt: number, liveReturnedAt: number | null): boolean {
  return liveReturnedAt === null || dataUpdatedAt >= liveReturnedAt;
}
