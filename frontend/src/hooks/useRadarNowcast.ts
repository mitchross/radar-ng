import { useQuery } from "@tanstack/react-query";
import { fetchRadarNowcast } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";

const REFRESH_MS = 60_000;

export function useRadarNowcast() {
  const latitude = useWeatherStore((state) => state.latitude);
  const longitude = useWeatherStore((state) => state.longitude);
  const serverUrl = useWeatherStore((state) => state.serverUrl);

  return useQuery({
    queryKey: ["radar-nowcast", latitude, longitude, serverUrl],
    queryFn: () => fetchRadarNowcast(serverUrl, latitude!, longitude!),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: REFRESH_MS,
    staleTime: REFRESH_MS,
    retry: 2,
  });
}
