import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";

export interface WindField {
  ok: true;
  timestamp: string;
  width: number;
  height: number;
  lat_min: number;
  lat_max: number;
  lon_min: number;
  lon_max: number;
  u_min: number;
  u_max: number;
  v_min: number;
  v_max: number;
  u: number[]; // int8 scaled; convert via u_min + (n + 127) / 254 * (u_max - u_min)
  v: number[];
}

interface WindFieldMissing {
  ok: false;
  reason: string;
}

/** Fetches the U/V wind vector field for a given HRRR timestamp. */
export function useWindField(timestamp: string | null) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  return useQuery({
    queryKey: ["wind-field", serverUrl, timestamp],
    queryFn: async (): Promise<WindField | WindFieldMissing> => {
      if (!timestamp) throw new Error("no timestamp");
      const r = await fetch(
        `${serverUrl}/api/wind-field/${encodeURIComponent(timestamp)}`,
      );
      if (!r.ok) throw new Error(`wind-field ${r.status}`);
      return r.json();
    },
    enabled: !!timestamp,
    staleTime: 15 * 60 * 1000,
    refetchInterval: false,
  });
}

/** Sample the wind field at a given lon/lat, returning (u, v) in mph. */
export function sampleWindField(field: WindField, lat: number, lon: number): [number, number] {
  "worklet";
  const fx = ((lon - field.lon_min) / (field.lon_max - field.lon_min)) * (field.width - 1);
  const fy = ((field.lat_max - lat) / (field.lat_max - field.lat_min)) * (field.height - 1);
  if (fx < 0 || fx > field.width - 1 || fy < 0 || fy > field.height - 1) {
    return [0, 0];
  }
  const x0 = Math.floor(fx);
  const y0 = Math.floor(fy);
  const x1 = Math.min(field.width - 1, x0 + 1);
  const y1 = Math.min(field.height - 1, y0 + 1);
  const dx = fx - x0;
  const dy = fy - y0;

  const uSpan = field.u_max - field.u_min;
  const vSpan = field.v_max - field.v_min;
  const unscaleU = (n: number) => field.u_min + ((n + 127) / 254) * uSpan;
  const unscaleV = (n: number) => field.v_min + ((n + 127) / 254) * vSpan;

  const i00 = y0 * field.width + x0;
  const i10 = y0 * field.width + x1;
  const i01 = y1 * field.width + x0;
  const i11 = y1 * field.width + x1;

  const u00 = unscaleU(field.u[i00]);
  const u10 = unscaleU(field.u[i10]);
  const u01 = unscaleU(field.u[i01]);
  const u11 = unscaleU(field.u[i11]);
  const v00 = unscaleV(field.v[i00]);
  const v10 = unscaleV(field.v[i10]);
  const v01 = unscaleV(field.v[i01]);
  const v11 = unscaleV(field.v[i11]);

  const u0 = u00 * (1 - dx) + u10 * dx;
  const u1 = u01 * (1 - dx) + u11 * dx;
  const v0 = v00 * (1 - dx) + v10 * dx;
  const v1 = v01 * (1 - dx) + v11 * dx;

  return [u0 * (1 - dy) + u1 * dy, v0 * (1 - dy) + v1 * dy];
}
