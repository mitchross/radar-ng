/**
 * The wind-field type and its bilinear sampler, shared by the web radar and the app.
 * The app runs the sampler on the UI thread inside a Reanimated worklet, so the function carries the
 * `'worklet'` directive (a no-op string on the web) and touches nothing outside its own body.
 */
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
  /** int8 scaled; convert via `u_min + (n + 127) / 254 * (u_max - u_min)`. -128 = no data. */
  u: number[];
  v: number[];
}

/** (u, v) in mph at a point, or null outside the field / model domain. */
export function sampleWind(f: WindField, lat: number, lon: number): [number, number] | null {
  "worklet";
  const fx = ((lon - f.lon_min) / (f.lon_max - f.lon_min)) * (f.width - 1);
  const fy = ((f.lat_max - lat) / (f.lat_max - f.lat_min)) * (f.height - 1);
  if (!(fx >= 0 && fx <= f.width - 1 && fy >= 0 && fy <= f.height - 1)) return null;
  const x0 = Math.floor(fx);
  const y0 = Math.floor(fy);
  const x1 = Math.min(f.width - 1, x0 + 1);
  const y1 = Math.min(f.height - 1, y0 + 1);
  const idx = [y0 * f.width + x0, y0 * f.width + x1, y1 * f.width + x0, y1 * f.width + x1];
  // -128 marks cells outside the model domain.
  if (idx.some((i) => f.u[i] === -128 || f.v[i] === -128)) return null;
  const dx = fx - x0;
  const dy = fy - y0;
  const w = [(1 - dx) * (1 - dy), dx * (1 - dy), (1 - dx) * dy, dx * dy];
  const un = (n: number) => f.u_min + ((n + 127) / 254) * (f.u_max - f.u_min);
  const vn = (n: number) => f.v_min + ((n + 127) / 254) * (f.v_max - f.v_min);
  let u = 0;
  let v = 0;
  idx.forEach((i, k) => {
    u += un(f.u[i]) * w[k];
    v += vn(f.v[i]) * w[k];
  });
  return [u, v];
}
