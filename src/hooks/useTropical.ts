import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { trace } from "../lib/telemetry";

type TropicalFeature = GeoJSON.Feature<
  GeoJSON.Point | GeoJSON.LineString | GeoJSON.Polygon,
  {
    kind: "position" | "track" | "cone";
    storm_id: string;
    name: string;
    classification?: string;
    wind_mph?: number;
    pressure_mb?: number;
  }
>;

export interface TropicalCollection {
  type: "FeatureCollection";
  features: TropicalFeature[];
  generated_at: number;
  storm_count?: number;
}

export function useTropical() {
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["tropical", serverUrl],
    queryFn: (): Promise<TropicalCollection> =>
      trace("api.fetchTropical", async (span) => {
        const r = await fetch(`${serverUrl}/api/tropical`);
        span.setAttribute("http.status_code", r.status);
        if (!r.ok) throw new Error(`tropical fetch ${r.status}`);
        const json = (await r.json()) as TropicalCollection;
        span.setAttribute("radar.tropical.storms", json.storm_count ?? json.features.length);
        return json;
      }),
    refetchInterval: 5 * 60_000,
    staleTime: 5 * 60_000,
  });
}
