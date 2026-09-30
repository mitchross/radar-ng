import { useQuery } from "@tanstack/react-query";
import { explainAlert } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";

/** Optional AI explanation for one alert; `null` (render nothing) when off or unavailable. */
export function useAlertExplanation(alertId: string | null | undefined) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const enabled = useWeatherStore((s) => s.aiNarration) && !!alertId;
  const query = useQuery({
    queryKey: ["alert-explain", alertId, serverUrl],
    queryFn: ({ signal }) => explainAlert(serverUrl, alertId!, signal),
    enabled,
    staleTime: 30 * 60_000,
    retry: false,
  });
  return { explanation: enabled ? (query.data ?? null) : null, loading: enabled && query.isLoading };
}
