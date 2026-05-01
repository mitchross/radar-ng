import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { trace } from "../lib/telemetry";

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

/** Rolling 15-min lightning GeoJSON from the self-hosted server. */
export function useLightning() {
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["lightning", serverUrl],
    queryFn: (): Promise<LightningCollection> =>
      trace("api.fetchLightning", async (span) => {
        const r = await fetch(`${serverUrl}/api/lightning`);
        span.setAttribute("http.status_code", r.status);
        if (!r.ok) throw new Error(`lightning fetch ${r.status}`);
        const json = (await r.json()) as LightningCollection;
        span.setAttribute("radar.lightning.count", json.features.length);
        return json;
      }),
    refetchInterval: 10_000,
    staleTime: 8_000,
  });
}
