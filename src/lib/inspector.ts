/**
 * Inspector/eyedropper client — asks the self-hosted tile-server for the
 * interpolated layer value at a point. Falls back to Open-Meteo for
 * temperature/wind so the feature still works on the free/public tier.
 */
import type { LayerType, Palette } from "../types/weather";

export interface InspectReading {
  ok: boolean;
  value: number | null;
  unit: string;
  source: "grid" | "forecast" | "unavailable";
  reason?: string;
}

interface InspectOptions {
  dataSource: "rainviewer" | "selfhosted";
  serverUrl: string;
  layer: LayerType;
  timestamp: string;
  lat: number;
  lon: number;
}

export async function inspectPoint(opts: InspectOptions): Promise<InspectReading> {
  // Self-hosted tile-server has the grid-backed endpoint.
  if (opts.dataSource === "selfhosted" && opts.serverUrl) {
    try {
      const url = `${opts.serverUrl}/api/inspect/${opts.layer}/${encodeURIComponent(opts.timestamp)}/${opts.lat}/${opts.lon}`;
      const resp = await fetch(url);
      if (resp.ok) {
        const json = await resp.json();
        if (json.ok && json.value != null) {
          return { ok: true, value: json.value, unit: json.unit ?? "", source: "grid" };
        }
        return {
          ok: false,
          value: null,
          unit: json.unit ?? "",
          source: "unavailable",
          reason: json.reason ?? "no_value",
        };
      }
    } catch {
      // fall through to forecast
    }
  }

  // Free-tier fallback: for temp/wind use Open-Meteo. Radar/reflectivity has
  // no free point endpoint — return unavailable.
  if (opts.layer === "temperature" || opts.layer === "wind") {
    try {
      const r = await fetch(
        `https://api.open-meteo.com/v1/forecast?latitude=${opts.lat}&longitude=${opts.lon}&current=temperature_2m,wind_speed_10m&temperature_unit=fahrenheit&wind_speed_unit=mph`,
      );
      const json = await r.json();
      if (opts.layer === "temperature" && json?.current?.temperature_2m != null) {
        return { ok: true, value: json.current.temperature_2m, unit: "°F", source: "forecast" };
      }
      if (opts.layer === "wind" && json?.current?.wind_speed_10m != null) {
        return { ok: true, value: json.current.wind_speed_10m, unit: "mph", source: "forecast" };
      }
    } catch {
      // fall through
    }
  }

  return { ok: false, value: null, unit: "", source: "unavailable", reason: "no_source" };
}

export function formatReading(layer: LayerType, r: InspectReading): string {
  if (!r.ok || r.value == null) return "\u2014";
  const v = r.value;

  if (layer === "radar" || layer === "radar-hrrr") {
    if (v < 5) return "No echo";
    const dbz = Math.round(v);
    return `${dbz} dBZ ${describeDBZ(dbz)}`;
  }
  if (layer === "temperature") return `${Math.round(v)}${r.unit || "°F"}`;
  if (layer === "wind") return `${Math.round(v)} ${r.unit || "mph"}`;
  if (layer === "cape") return `${Math.round(v)} ${r.unit || "J/kg"}`;
  if (layer === "precip-type") return "Active";
  if (layer === "precip-accum") return v < 0.01 ? "—" : `${v.toFixed(2)} in`;
  if (layer === "cloud") return `${Math.round(v)}%`;
  return `${v.toFixed(1)} ${r.unit}`.trim();
}

function describeDBZ(dbz: number): string {
  if (dbz < 15) return "Drizzle";
  if (dbz < 25) return "Light";
  if (dbz < 35) return "Moderate";
  if (dbz < 45) return "Heavy";
  if (dbz < 55) return "Intense";
  return "Extreme";
}

/** Unused for now; kept for when the eyedropper moves to tile-URL-based sampling. */
export function gridUrlFor(
  serverUrl: string,
  layer: LayerType,
  timestamp: string,
  _palette: Palette,
): string {
  return `${serverUrl}/data/grids/${layer}/${encodeURIComponent(timestamp)}.bin`;
}
