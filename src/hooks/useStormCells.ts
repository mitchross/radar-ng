import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";

export interface StormCell {
  type: "Feature";
  geometry: { type: "Point"; coordinates: [number, number] };
  properties: {
    cell_id: number;
    peak_dbz: number;
    area_km2: number;
    pixel_count: number;
  };
}

export interface StormCellCollection {
  type: "FeatureCollection";
  features: StormCell[];
  generated_at?: number;
  timestamp?: string;
  cell_count?: number;
  threshold_dbz?: number;
}

export function useStormCells() {
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["storms", serverUrl],
    queryFn: async (): Promise<StormCellCollection> => {
      const r = await fetch(`${serverUrl}/api/storms`);
      if (!r.ok) throw new Error(`storms fetch ${r.status}`);
      return r.json();
    },
    enabled: dataSource === "selfhosted",
    refetchInterval: 60_000,
    staleTime: 60_000,
  });
}
