import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { trace } from "../lib/telemetry";

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
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["storms", serverUrl],
    queryFn: (): Promise<StormCellCollection> =>
      trace("api.fetchStormCells", async (span) => {
        const r = await fetch(`${serverUrl}/api/storms`);
        span.setAttribute("http.status_code", r.status);
        if (!r.ok) throw new Error(`storms fetch ${r.status}`);
        const json = (await r.json()) as StormCellCollection;
        span.setAttribute("radar.storms.count", json.cell_count ?? json.features.length);
        return json;
      }),
    refetchInterval: 60_000,
    staleTime: 60_000,
  });
}
