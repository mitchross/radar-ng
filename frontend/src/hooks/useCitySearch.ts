import { useQuery } from "@tanstack/react-query";
import { searchCities } from "../lib/geocoding";

export function useCitySearch(query: string) {
  const normalized = query.trim();
  return useQuery({
    queryKey: ["city-search", normalized],
    queryFn: () => searchCities(normalized),
    enabled: normalized.length >= 2,
    staleTime: 5 * 60 * 1000,
  });
}
