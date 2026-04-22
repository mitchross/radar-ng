import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";

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
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["tropical", serverUrl],
    queryFn: async (): Promise<TropicalCollection> => {
      const r = await fetch(`${serverUrl}/api/tropical`);
      if (!r.ok) throw new Error(`tropical fetch ${r.status}`);
      return r.json();
    },
    enabled: dataSource === "selfhosted",
    refetchInterval: 5 * 60_000,
    staleTime: 5 * 60_000,
  });
}
