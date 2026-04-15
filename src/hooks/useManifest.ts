import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchRadarManifest, fetchSelfHostedManifest } from "../lib/api";
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

  const rainviewerQuery = useQuery({
    queryKey: ["radar-manifest-rainviewer"],
    queryFn: fetchRadarManifest,
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
    enabled: dataSource === "rainviewer",
  });

  const selfHostedQuery = useQuery({
    queryKey: ["manifest-selfhosted", serverUrl],
    queryFn: () => fetchSelfHostedManifest(serverUrl),
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
    enabled: dataSource === "selfhosted",
  });

  useEffect(() => {
    let allFrames: RadarFrame[] = [];

    if (dataSource === "rainviewer" && rainviewerQuery.data) {
      allFrames = [
        ...rainviewerQuery.data.radar.past,
        ...rainviewerQuery.data.radar.nowcast,
      ];
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
  }, [rainviewerQuery.data, selfHostedQuery.data, dataSource, activeLayer]);

  return dataSource === "rainviewer" ? rainviewerQuery : selfHostedQuery;
}
