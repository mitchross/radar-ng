import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { trace } from "../lib/telemetry";
import { fetchWithTimeout } from "../lib/api";
import { hourKey } from "../lib/timeKeys";
import { sampleWind, type WindField } from "../../../web/src/shared/windSample";

export type { WindField };

interface WindFieldMissing {
  ok: false;
  reason: string;
}

/**
 * Fetches the U/V wind vector field for the HRRR hour containing `timestamp`.
 * Keyed by hour: HRRR grids are hourly and the server resolves any unmatched
 * timestamp to the latest grid anyway, so per-frame keys (2-min MRMS) only
 * re-downloaded the same ~70 kB field on every playback tick.
 */
export function useWindField(timestamp: string | null) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const hour = timestamp ? hourKey(timestamp) : null;

  return useQuery({
    queryKey: ["wind-field", serverUrl, hour],
    queryFn: ({ signal }): Promise<WindField | WindFieldMissing> =>
      trace(
        "api.fetchWindField",
        async (span) => {
          if (!hour) throw new Error("no timestamp");
          const r = await fetchWithTimeout(
            `${serverUrl}/api/wind-field/${encodeURIComponent(hour)}`,
            {},
            signal,
          );
          span.setAttribute("http.status_code", r.status);
          if (!r.ok) throw new Error(`wind-field ${r.status}`);
          return r.json();
        },
        { "radar.timestamp": hour ?? "" },
      ),
    enabled: !!hour,
    staleTime: 15 * 60 * 1000,
    refetchInterval: false,
  });
}

/** Sample the wind field at a given lon/lat, returning (u, v) in mph. */
export function sampleWindField(field: WindField, lat: number, lon: number): [number, number] {
  "worklet";
  // Outside the field the particles stand still; the shared sampler says so with null.
  return sampleWind(field, lat, lon) ?? [0, 0];
}
