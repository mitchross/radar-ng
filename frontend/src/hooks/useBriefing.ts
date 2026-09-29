import { useQuery } from "@tanstack/react-query";
import { fetchBriefing } from "../lib/api";
import { locationKey, PRECISION } from "../lib/coordinates";
import { useWeatherStore } from "../stores/useWeatherStore";

const TEN_MINUTES = 10 * 60_000;

/** Optional AI briefing for the active place; `null` means "show nothing". */
export function useBriefing(placeName: string) {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const enabled = useWeatherStore((s) => s.aiNarration) && latitude != null && longitude != null;
  const position = latitude != null && longitude != null ? locationKey(latitude, longitude, PRECISION.WEATHER) : null;
  const query = useQuery({
    queryKey: ["briefing", position, placeName, serverUrl],
    queryFn: ({ signal }) => fetchBriefing(serverUrl, latitude!, longitude!, placeName, signal),
    enabled,
    staleTime: TEN_MINUTES,
    refetchInterval: TEN_MINUTES,
    retry: false,
  });
  return enabled ? (query.data ?? null) : null;
}
