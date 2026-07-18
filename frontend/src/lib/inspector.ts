/**
 * Inspector/eyedropper client — asks the self-hosted tile-server for the
 * interpolated layer value at a point.
 */
import type { LayerType, Palette } from "../types/weather";
import { trace } from "./telemetry";

export interface InspectReading {
  ok: boolean;
  value: number | null;
  unit: string;
  source: "grid" | "unavailable";
  reason?: string;
}

interface InspectOptions {
  serverUrl: string;
  layer: LayerType;
  timestamp: string;
  lat: number;
  lon: number;
}

export async function inspectPoint(opts: InspectOptions): Promise<InspectReading> {
  return trace(
    "api.inspectPoint",
    async (span) => {
      try {
        const url = `${opts.serverUrl}/api/inspect/${opts.layer}/${encodeURIComponent(opts.timestamp)}/${opts.lat}/${opts.lon}`;
        const resp = await fetch(url);
        span.setAttribute("http.status_code", resp.status);
        if (resp.ok) {
          const json = await resp.json();
          if (json.ok && json.value != null) {
            span.setAttribute("inspector.value", json.value);
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
      } catch (err) {
        span.recordException(err as Error);
      }
      return { ok: false, value: null, unit: "", source: "unavailable", reason: "no_source" };
    },
    {
      "inspector.layer": opts.layer,
      "inspector.timestamp": opts.timestamp,
      "geo.lat": opts.lat,
      "geo.lon": opts.lon,
    },
  );
}

export function formatReading(layer: LayerType, r: InspectReading): string {
  if (!r.ok || r.value == null) return "\u2014";
  const v = r.value;

  if (layer === "radar" || layer === "radar-hrrr") {
    if (v < 5) return "Clear";
    return describeDBZ(Math.round(v));
  }
  if (layer === "temperature") return `${Math.round(v)}${r.unit || "°F"}`;
  if (layer === "wind") return `${Math.round(v)} ${r.unit || "mph"}`;
  if (layer === "cape") return `${Math.round(v)} ${r.unit || "J/kg"}`;
  if (layer === "precip-type") return "Active";
  if (layer === "precip-accum") return v < 0.01 ? "—" : `${v.toFixed(2)} in`;
  if (layer === "cloud") return `${Math.round(v)}%`;
  if (layer === "air-quality") return `${Math.round(v)} µg/m³ · ${describePm25(v)}`;
  if (layer === "ozone") return `${Math.round(v)} ppb · ${describeOzone(v)}`;
  return `${v.toFixed(1)} ${r.unit}`.trim();
}

// EPA 2024 PM2.5 breakpoints (µg/m³) → AQI category.
function describePm25(ugm3: number): string {
  if (ugm3 <= 9) return "Good";
  if (ugm3 <= 35.4) return "Moderate";
  if (ugm3 <= 55.4) return "Sensitive";
  if (ugm3 <= 125.4) return "Unhealthy";
  if (ugm3 <= 225.4) return "Very Unhealthy";
  return "Hazardous";
}

// 8-hour ozone breakpoints (ppb) → AQI category, applied to the hourly value.
function describeOzone(ppb: number): string {
  if (ppb <= 54) return "Good";
  if (ppb <= 70) return "Moderate";
  if (ppb <= 85) return "Sensitive";
  if (ppb <= 105) return "Unhealthy";
  if (ppb <= 200) return "Very Unhealthy";
  return "Hazardous";
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
