import { useEffect } from "react";
import { useStormCells } from "./useStormCells";
import { parseStormCells } from "../lib/stormTracks";
import { checkStormNotice } from "../lib/stormNoticeScheduler";
import { useWeatherStore } from "../stores/useWeatherStore";

/**
 * While the app is open and the notice is on, poll the tracked storms and post
 * the "storm may reach you" notification once per cell. Mount once at the root.
 */
export function useStormNotice(): void {
  const enabled = useWeatherStore((s) => s.stormNoticeEnabled);
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const { data } = useStormCells(enabled && latitude != null && longitude != null);

  useEffect(() => {
    if (!enabled || !data || latitude == null || longitude == null) return;
    void checkStormNotice(parseStormCells(data), [longitude, latitude]);
  }, [enabled, data, latitude, longitude]);
}
