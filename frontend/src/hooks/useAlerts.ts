import { useQuery } from "@tanstack/react-query";
import { fetchAlerts } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useAlerts() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);

  return useQuery({
    queryKey: ["alerts", latitude, longitude],
    queryFn: () => fetchAlerts(latitude!, longitude!),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: DEFAULTS.ALERTS_REFETCH_MS,
    staleTime: DEFAULTS.ALERTS_REFETCH_MS,
  });
}
