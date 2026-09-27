import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { useManifestQuery } from "./useManifest";
import { inspectPoint } from "../lib/inspector";
import { locationKey, PRECISION } from "../lib/coordinates";
import { nearestFrameTimestamp } from "../lib/airQuality";
import { useNow } from "./useNow";

const REFRESH_MS = 15 * 60_000;

export interface AirQualityNow {
  pm25: number | null;
  ozonePpb: number | null;
}

/**
 * PM2.5 and ozone at the active location, sampled from the air-quality
 * frames nearest to now. Null data when there's no location or no frame
 * within 90 minutes of now (a stale AQM run is not "current").
 */
export function useAirQualityNow() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const manifest = useManifestQuery().data;
  // Re-picks the nearest hourly frame as time passes.
  const now = useNow(REFRESH_MS);
  const pmTs = nearestFrameTimestamp(manifest?.layers?.["air-quality"]?.frames, now);
  const o3Ts = nearestFrameTimestamp(manifest?.layers?.ozone?.frames, now);
  const position =
    latitude != null && longitude != null ? locationKey(latitude, longitude, PRECISION.WEATHER) : null;

  return useQuery({
    queryKey: ["air-quality-now", position, pmTs, o3Ts, serverUrl],
    enabled: position !== null && (pmTs !== null || o3Ts !== null),
    staleTime: REFRESH_MS,
    retry: 1,
    queryFn: async ({ signal }): Promise<AirQualityNow> => {
      const read = async (layer: "air-quality" | "ozone", timestamp: string | null) => {
        if (!timestamp) return null;
        const r = await inspectPoint({ serverUrl, layer, timestamp, lat: latitude!, lon: longitude!, signal });
        return r.ok ? r.value : null;
      };
      const [pm25, ozonePpb] = await Promise.all([read("air-quality", pmTs), read("ozone", o3Ts)]);
      return { pm25, ozonePpb };
    },
  });
}
