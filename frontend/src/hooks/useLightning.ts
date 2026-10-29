import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { trace } from "../lib/telemetry";
import { fetchWithTimeout } from "../lib/api";

interface LightningStrike {
  type: "Feature";
  geometry: { type: "Point"; coordinates: [number, number] };
  properties: { time: number; age_s: number; polarity: number; mds: number };
}

export interface LightningCollection {
  type: "FeatureCollection";
  features: LightningStrike[];
  generated_at: number;
  retention_min?: number;
}

/**
 * The server stamps `age_s` when it flushes the file, so a stalled worker leaves
 * it frozen. Recompute it from the strike time, like web/src/overlays.ts does.
 */
export function withClientAge(fc: LightningCollection, nowSec: number): LightningCollection {
  return {
    ...fc,
    features: fc.features.map((f) => ({
      ...f,
      properties: { ...f.properties, age_s: Math.max(0, nowSec - Number(f.properties?.time ?? nowSec)) },
    })),
  };
}

/** Rolling 15-min lightning GeoJSON from the self-hosted server. `enabled` gates polling (extras toggle). */
export function useLightning(enabled = true) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["lightning", serverUrl],
    enabled,
    queryFn: ({ signal }): Promise<LightningCollection> =>
      trace("api.fetchLightning", async (span) => {
        const r = await fetchWithTimeout(`${serverUrl}/api/lightning`, {}, signal);
        span.setAttribute("http.status_code", r.status);
        if (!r.ok) throw new Error(`lightning fetch ${r.status}`);
        const json = (await r.json()) as LightningCollection;
        span.setAttribute("radar.lightning.count", json.features.length);
        return withClientAge(json, Date.now() / 1000);
      }),
    refetchInterval: 10_000,
    // Honoured only because _layout.tsx wires focusManager to AppState.
    refetchIntervalInBackground: false,
    staleTime: 8_000,
  });
}
