import { useQuery } from "@tanstack/react-query";
import { searchCities } from "../lib/geocoding";
import { useWeatherStore } from "../stores/useWeatherStore";

export function useCitySearch(query: string) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const normalized = query.trim();
  return useQuery({
    queryKey: ["city-search", serverUrl, normalized],
    queryFn: ({ signal }) => searchCities(serverUrl, normalized, signal),
    enabled: normalized.length >= 2,
    staleTime: 5 * 60 * 1000,
  });
}
