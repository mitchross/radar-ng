import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchRadarManifest } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useManifest() {
  const setFrames = useWeatherStore((s) => s.setFrames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);

  const query = useQuery({
    queryKey: ["radar-manifest"],
    queryFn: fetchRadarManifest,
    refetchInterval: DEFAULTS.MANIFEST_REFETCH_MS,
  });

  useEffect(() => {
    if (!query.data) return;
    const allFrames = [
      ...query.data.radar.past,
      ...query.data.radar.nowcast,
    ];
    setFrames(allFrames);
    if (currentFrameIndex === -1 || currentFrameIndex >= allFrames.length) {
      setCurrentFrameIndex(allFrames.length - 1);
    }
  }, [query.data]);

  return query;
}
