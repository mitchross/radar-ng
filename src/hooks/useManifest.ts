import { useEffect, useCallback, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { buildIEMFrames, fetchSelfHostedManifest } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";
import type { RadarFrame } from "../types/weather";

export function useManifest() {
  const setFrames = useWeatherStore((s) => s.setFrames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);

  // IEM frames are deterministic — no API call needed
  const [iemTick, setIemTick] = useState(0);

  useEffect(() => {
    if (dataSource !== "rainviewer") return;
    const interval = setInterval(() => setIemTick((t) => t + 1), DEFAULTS.MANIFEST_REFETCH_MS);
    return () => clearInterval(interval);
  }, [dataSource]);

  const selfHostedQuery = useQuery({
    queryKey: ["manifest-selfhosted", serverUrl],
    queryFn: () => fetchSelfHostedManifest(serverUrl),
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
    enabled: dataSource === "selfhosted",
  });

  useEffect(() => {
    let allFrames: RadarFrame[] = [];

    if (dataSource === "rainviewer") {
      allFrames = buildIEMFrames();
    } else if (dataSource === "selfhosted" && selfHostedQuery.data) {
      const layerKey = activeLayer === "radar" ? "radar" : activeLayer;
      const layerData = selfHostedQuery.data.layers[layerKey];
      if (layerData) {
        allFrames = layerData.timestamps.map((ts) => ({
          time: Math.floor(new Date(ts).getTime() / 1000),
          path: ts,
        }));
      }
    }

    if (allFrames.length > 0) {
      setFrames(allFrames);
      if (currentFrameIndex === -1 || currentFrameIndex >= allFrames.length) {
        setCurrentFrameIndex(allFrames.length - 1);
      }
    }
  }, [iemTick, selfHostedQuery.data, dataSource, activeLayer]);

  return selfHostedQuery;
}
