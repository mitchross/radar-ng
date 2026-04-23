import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchSelfHostedManifest } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";
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
    return entry.timestamps.map((ts) => ({
      time: Math.floor(new Date(ts).getTime() / 1000),
      path: ts,
      ...(source ? { source } : {}),
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

export function useManifest() {
  const setFrames = useWeatherStore((s) => s.setFrames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const timelineMode = useWeatherStore((s) => s.timelineMode);

  const query = useQuery({
    queryKey: ["manifest", serverUrl],
    queryFn: () => fetchSelfHostedManifest(serverUrl),
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
  });

  useEffect(() => {
    if (!query.data) return;
    const frames = buildSelfHostedFrames(query.data, activeLayer, timelineMode);
    if (frames.length === 0) return;
    setFrames(frames);
    if (currentFrameIndex === -1 || currentFrameIndex >= frames.length) {
      const nowSec = Math.floor(Date.now() / 1000);
      let best = 0;
      let bestDiff = Infinity;
      for (let i = 0; i < frames.length; i++) {
        const d = Math.abs(frames[i].time - nowSec);
        if (d < bestDiff) {
          bestDiff = d;
          best = i;
        }
      }
      setCurrentFrameIndex(best);
    }
  }, [query.data, activeLayer, timelineMode]);

  return query;
}
