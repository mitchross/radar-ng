import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";

interface LightningStrike {
  type: "Feature";
  geometry: { type: "Point"; coordinates: [number, number] };
  properties: { time: number; age_s: number; polarity: number; mds: number };
}

interface LightningCollection {
  type: "FeatureCollection";
  features: LightningStrike[];
  generated_at: number;
  retention_min?: number;
}

/** Fetches the rolling 15-min lightning GeoJSON from the self-hosted server.
 *  No-ops on the free/public tier — lightning requires self-hosted backend. */
export function useLightning() {
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["lightning", serverUrl],
    queryFn: async (): Promise<LightningCollection> => {
      const r = await fetch(`${serverUrl}/api/lightning`);
      if (!r.ok) throw new Error(`lightning fetch ${r.status}`);
      return r.json();
    },
    enabled: dataSource === "selfhosted",
    refetchInterval: 10_000,
    staleTime: 8_000,
  });
}
