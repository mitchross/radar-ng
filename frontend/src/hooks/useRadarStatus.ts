import { useMemo } from "react";
import { useNow } from "./useNow";
import { pickNowFrameIndex, useManifestQuery } from "./useManifest";
import { DEFAULTS } from "../lib/constants";
import { radarQueryIsOffline, radarStatus, type RadarStatus } from "../lib/radarStatus";
import type { RadarFrame } from "../types/weather";

export type RadarLayerKey = "radar" | "radar-composite";

export interface RadarStatusView {
  status: RadarStatus;
  /** The latest observed radar frame, the one the status describes. */
  nowFrame: RadarFrame | null;
  layerKey: RadarLayerKey;
  /** True until the first manifest (network or disk cache) has arrived. */
  pending: boolean;
  refetch: () => void;
}

/**
 * One truthful radar freshness status for every surface (Home mini-map, Radar
 * tab). Derived from the observed MRMS layer, so a forecast-only layer on the
 * Radar tab still reports the server's observation health.
 */
export function useRadarStatus(): RadarStatusView {
  const { data: manifest, dataUpdatedAt, isError, isPaused, isPending, refetch } = useManifestQuery();

  // Match the dedicated Radar tab's observed MRMS source first. Composite is
  // only a compatibility fallback for clusters that still publish it.
  const layerKey: RadarLayerKey = manifest?.layers?.radar
    ? "radar"
    : manifest?.layers?.["radar-composite"]
      ? "radar-composite"
      : "radar";
  const layer = manifest?.layers?.[layerKey];
  const frames = useMemo<RadarFrame[]>(() => (
    (layer?.frames ?? layer?.timestamps.map((timestamp) => ({ timestamp, path: timestamp })) ?? []).map((frame) => ({
      time: Math.floor(new Date(frame.timestamp).getTime() / 1000),
      timestamp: frame.timestamp,
      path: frame.path,
      source: "radar",
    }))
  ), [layer?.frames, layer?.timestamps]);
  const nowFrameIndex = pickNowFrameIndex(frames);
  const nowFrame = nowFrameIndex >= 0 ? frames[nowFrameIndex] : null;

  // The clock ticks with the manifest poll, and dataUpdatedAt moves it forward
  // on every successful poll, so LIVE can't stay frozen during an ingest stall.
  const clock = useNow(DEFAULTS.MANIFEST_REFETCH_MS);
  const status = radarStatus({
    frameTimeSeconds: nowFrame?.time ?? null,
    refreshFailed: radarQueryIsOffline(isError, isPaused),
    loading: isPending,
    nowMilliseconds: Math.max(clock, dataUpdatedAt),
  });

  return { status, nowFrame, layerKey, pending: isPending, refetch: () => void refetch() };
}
