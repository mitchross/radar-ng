import { useQuery } from "@tanstack/react-query";
import { fetchForecast } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useForecast() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["forecast", latitude, longitude, dataSource, serverUrl],
    queryFn: () =>
      fetchForecast(latitude!, longitude!, { dataSource, serverUrl }),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: DEFAULTS.FORECAST_REFETCH_MS,
    staleTime: DEFAULTS.FORECAST_REFETCH_MS,
  });
}
