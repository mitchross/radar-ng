import { useQuery } from "@tanstack/react-query";
import { fetchRadarNowcast } from "../lib/api";
import { locationKey, PRECISION } from "../lib/coordinates";
import { useWeatherStore } from "../stores/useWeatherStore";

const REFRESH_MS = 60_000;

export function useRadarNowcast() {
  const latitude = useWeatherStore((state) => state.latitude);
  const longitude = useWeatherStore((state) => state.longitude);
  const serverUrl = useWeatherStore((state) => state.serverUrl);
  const position =
    latitude != null && longitude != null
      ? locationKey(latitude, longitude, PRECISION.WEATHER)
      : null;

  return useQuery({
    queryKey: ["radar-nowcast", position, serverUrl],
    queryFn: ({ signal }) => fetchRadarNowcast(serverUrl, latitude!, longitude!, signal),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: REFRESH_MS,
    staleTime: REFRESH_MS,
    retry: 2,
  });
}
