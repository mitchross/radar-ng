import { useQuery } from "@tanstack/react-query";
import { fetchForecast } from "../lib/api";
import { locationKey, PRECISION } from "../lib/coordinates";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useForecast() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const position =
    latitude != null && longitude != null
      ? locationKey(latitude, longitude, PRECISION.WEATHER)
      : null;

  return useQuery({
    queryKey: ["forecast", position, serverUrl],
    queryFn: ({ signal }) => fetchForecast(serverUrl, latitude!, longitude!, signal),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: DEFAULTS.FORECAST_REFETCH_MS,
    staleTime: DEFAULTS.FORECAST_REFETCH_MS,
  });
}
