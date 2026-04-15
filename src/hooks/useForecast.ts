import { useQuery } from "@tanstack/react-query";
import { fetchForecast } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useForecast() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);

  return useQuery({
    queryKey: ["forecast", latitude, longitude],
    queryFn: () => fetchForecast(latitude!, longitude!),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: DEFAULTS.FORECAST_REFETCH_MS,
    staleTime: DEFAULTS.FORECAST_REFETCH_MS,
  });
}
