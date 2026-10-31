import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { trace } from "../lib/telemetry";
import { fetchWithTimeout } from "../lib/api";
import { hourKey } from "../lib/timeKeys";
import type { WindField } from "../../../web/src/shared/windSample";

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

/**
 * Sample the wind field at a given lon/lat, returning (u, v) in mph, or (0, 0) outside the field or
 * the model domain so particles stand still there.
 *
 * Runs on the UI thread inside a worklet, per particle per frame, so it stays self-contained on
 * purpose: no call into another module's function (an imported worklet crossing the worklet boundary
 * is the suspected cause of the Air Quality crash, #215) and no arrays or closures allocated per
 * call. The math matches `sampleWind` in web/src/shared/windSample.ts, which the web radar keeps
 * using; __tests__/lib/windField.test.ts asserts the two agree.
 */
export function sampleWindField(field: WindField, lat: number, lon: number): [number, number] {
  "worklet";
  const width = field.width;
  const height = field.height;
  const fx = ((lon - field.lon_min) / (field.lon_max - field.lon_min)) * (width - 1);
  const fy = ((field.lat_max - lat) / (field.lat_max - field.lat_min)) * (height - 1);
  if (!(fx >= 0 && fx <= width - 1 && fy >= 0 && fy <= height - 1)) return [0, 0];
  const x0 = Math.floor(fx);
  const y0 = Math.floor(fy);
  const x1 = Math.min(width - 1, x0 + 1);
  const y1 = Math.min(height - 1, y0 + 1);
  const i00 = y0 * width + x0;
  const i10 = y0 * width + x1;
  const i01 = y1 * width + x0;
  const i11 = y1 * width + x1;
  const us = field.u;
  const vs = field.v;
  // -128 marks cells outside the model domain.
  if (
    us[i00] === -128 || vs[i00] === -128 ||
    us[i10] === -128 || vs[i10] === -128 ||
    us[i01] === -128 || vs[i01] === -128 ||
    us[i11] === -128 || vs[i11] === -128
  ) {
    return [0, 0];
  }
  const dx = fx - x0;
  const dy = fy - y0;
  const w00 = (1 - dx) * (1 - dy);
  const w10 = dx * (1 - dy);
  const w01 = (1 - dx) * dy;
  const w11 = dx * dy;
  const uSpan = field.u_max - field.u_min;
  const vSpan = field.v_max - field.v_min;
  // Same accumulation order as the shared sampler, so the two agree to the last bit.
  let u = 0;
  let v = 0;
  u += (field.u_min + ((us[i00] + 127) / 254) * uSpan) * w00;
  v += (field.v_min + ((vs[i00] + 127) / 254) * vSpan) * w00;
  u += (field.u_min + ((us[i10] + 127) / 254) * uSpan) * w10;
  v += (field.v_min + ((vs[i10] + 127) / 254) * vSpan) * w10;
  u += (field.u_min + ((us[i01] + 127) / 254) * uSpan) * w01;
  v += (field.v_min + ((vs[i01] + 127) / 254) * vSpan) * w01;
  u += (field.u_min + ((us[i11] + 127) / 254) * uSpan) * w11;
  v += (field.v_min + ((vs[i11] + 127) / 254) * vSpan) * w11;
  return [u, v];
}
