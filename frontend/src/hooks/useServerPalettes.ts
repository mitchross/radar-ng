import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { fetchWithTimeout } from "../lib/api";
import type { ServerPalettes } from "../lib/serverPalettes";

/** The tile renderer's palette tables. They change only with a server release. */
export function useServerPalettes() {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  return useQuery({
    queryKey: ["palettes", serverUrl],
    queryFn: async ({ signal }): Promise<ServerPalettes> => {
      const r = await fetchWithTimeout(`${serverUrl}/api/palettes`, {}, signal);
      if (!r.ok) throw new Error(`palettes ${r.status}`);
      return r.json();
    },
    staleTime: 60 * 60 * 1000,
    refetchInterval: false,
  });
}
