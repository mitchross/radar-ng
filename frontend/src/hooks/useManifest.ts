import { useEffect, useMemo, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchSelfHostedManifest } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";
import { getString, setString } from "../lib/storage";
import { resnapFrameIndex } from "../lib/frameIndex";
import type { RadarFrame, SelfHostedManifest } from "../types/weather";

/**
 * Builds the frame list for the active layer + timeline mode.
 *
 *   mode=current  → past MRMS only (radar/radar-hrrr fall back to radar
 *                   since MRMS is the observation source)
 *   mode=forecast → past MRMS (last hour) + nowcast (0..+60min) + HRRR (+1h..+48h)
 *
 * Non-radar layers (temperature/wind/cape/precip-type) always source from
 * their own HRRR series since we don't observe them in real time.
 */
function buildSelfHostedFrames(
  manifest: SelfHostedManifest,
  layer: string,
  mode: "current" | "forecast",
): RadarFrame[] {
  const toFrames = (layerKey: string, source?: RadarFrame["source"]): RadarFrame[] => {
    const entry = manifest.layers[layerKey];
    if (!entry) return [];
    const manifestFrames: NonNullable<typeof entry.frames> = entry.frames ?? entry.timestamps.map((timestamp) => ({
      timestamp,
      path: timestamp,
    }));
    return manifestFrames.map((frame) => ({
      time: Math.floor(new Date(frame.timestamp).getTime() / 1000),
      timestamp: frame.timestamp,
      path: frame.path,
      ...(source ? { source } : {}),
      ...(frame.kind ? { kind: frame.kind } : {}),
      ...(frame.issued_at ? { issuedAt: frame.issued_at } : {}),
      ...(frame.lead_minutes !== undefined ? { leadMinutes: frame.lead_minutes } : {}),
      ...(frame.spatial_resolution_km !== undefined
        ? { spatialResolutionKm: frame.spatial_resolution_km }
        : {}),
      ...(frame.max_zoom !== undefined ? { maxZoom: frame.max_zoom } : {}),
    }));
  };

  const nowSec = Math.floor(Date.now() / 1000);

  if (layer === "radar" || layer === "radar-hrrr") {
    if (mode === "current") {
      return toFrames("radar", "radar");
    }
    const past = toFrames("radar", "radar").filter((f) => f.time <= nowSec);
    const nowcast = toFrames("nowcast", "nowcast").filter((f) => f.time > nowSec && f.time <= nowSec + 60 * 60);
    const hrrr = toFrames("radar-hrrr", "radar-hrrr").filter((f) => f.time > nowSec + 60 * 60);
    return dedupe([...past, ...nowcast, ...hrrr].sort((a, b) => a.time - b.time));
  }

  return toFrames(layer);
}

function dedupe(frames: RadarFrame[]): RadarFrame[] {
  const seen = new Set<number>();
  const out: RadarFrame[] = [];
  for (const f of frames) {
    if (seen.has(f.time)) continue;
    seen.add(f.time);
    out.push(f);
  }
  return out;
}

/**
 * Picks the "Now" frame: the most recent OBSERVED frame (source !== nowcast/HRRR)
 * with `time <= now`. Falls back to the closest-to-now frame if no observation
 * exists (e.g. on a forecast-only layer like temperature). Returns -1 when the
 * frame list is empty.
 */
export function pickNowFrameIndex(frames: RadarFrame[]): number {
  if (frames.length === 0) return -1;
  const nowSec = Math.floor(Date.now() / 1000);
  for (let i = frames.length - 1; i >= 0; i--) {
    const f = frames[i];
    const isObserved = f.source !== "nowcast" && f.source !== "radar-hrrr";
    if (isObserved && f.time <= nowSec) return i;
  }
  let best = 0;
  let bestDiff = Math.abs(frames[0].time - nowSec);
  for (let i = 1; i < frames.length; i++) {
    const d = Math.abs(frames[i].time - nowSec);
    if (d < bestDiff) {
      bestDiff = d;
      best = i;
    }
  }
  return best;
}

const MANIFEST_CACHE_KEY = "manifest-cache-v2";

function readCachedManifest(serverUrl: string): SelfHostedManifest | undefined {
  const cached = getString(MANIFEST_CACHE_KEY, "");
  if (!cached) return undefined;
  try {
    const parsed = JSON.parse(cached) as { serverUrl?: string; manifest?: SelfHostedManifest };
    return parsed.serverUrl === serverUrl ? parsed.manifest : undefined;
  } catch {
    return undefined;
  }
}

/**
 * The one manifest query. Radar tab, home mini-map and Settings all observe
 * this key, so one 30 s poll feeds all three (react-query dedupes by key).
 */
export function useManifestQuery() {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  return useQuery({
    queryKey: ["manifest", serverUrl],
    queryFn: async ({ signal }) => {
      const manifest = await fetchSelfHostedManifest(serverUrl, signal);
      setString(MANIFEST_CACHE_KEY, JSON.stringify({ serverUrl, manifest }));
      return manifest;
    },
    initialData: () => readCachedManifest(serverUrl),
    // The MMKV seed is stale by definition; without this react-query trusts it for a full 30 s.
    initialDataUpdatedAt: 0,
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
    refetchIntervalInBackground: false,
  });
}

/** Manifest query + writes the active layer's frame list into the store. Mount once (radar tab). */
export function useManifest() {
  const setFrames = useWeatherStore((s) => s.setFrames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const timelineMode = useWeatherStore((s) => s.timelineMode);

  const query = useManifestQuery();

  const frames = useMemo(
    () => (query.data ? buildSelfHostedFrames(query.data, activeLayer, timelineMode) : []),
    [query.data, activeLayer, timelineMode],
  );

  useEffect(() => {
    setFrames(frames);
  }, [frames, setFrames]);

  // Wall-clock time of the frame on screen. Frame identity across polls is
  // `time`, not index: a poll that prunes the head shifts every index.
  const shownTimeRef = useRef<number | null>(null);

  useEffect(() => {
    if (frames.length === 0) return;
    const idx = useWeatherStore.getState().currentFrameIndex;
    const byTime = resnapFrameIndex(frames, idx, shownTimeRef.current);
    const next = byTime === -1 ? pickNowFrameIndex(frames) : byTime;
    shownTimeRef.current = frames[next]?.time ?? null;
    if (next !== idx) setCurrentFrameIndex(next);
  }, [frames, setCurrentFrameIndex]);

  // Scrub / tick / explicit -1 reset: remember what's showing, and snap an
  // uninitialised or out-of-range index to "now".
  useEffect(() => {
    if (frames.length === 0) return;
    if (currentFrameIndex === -1 || currentFrameIndex >= frames.length) {
      setCurrentFrameIndex(pickNowFrameIndex(frames));
      return;
    }
    shownTimeRef.current = frames[currentFrameIndex].time;
  }, [frames, currentFrameIndex, setCurrentFrameIndex]);

  return query;
}
