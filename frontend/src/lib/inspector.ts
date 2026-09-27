/**
 * Inspector/eyedropper client — asks the self-hosted tile-server for the
 * interpolated layer value at a point.
 */
import type { LayerType } from "../types/weather";
import { PRECISION, roundCoords } from "./coordinates";
import { recordSpanError, trace } from "./telemetry";
import { fetchRadarNowcast, fetchWithTimeout } from "./api";

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
  signal?: AbortSignal;
}

export async function inspectPoint(opts: InspectOptions): Promise<InspectReading> {
  return trace(
    "api.inspectPoint",
    async (span) => {
      try {
        const { lat, lon } = roundCoords(opts.lat, opts.lon, PRECISION.POINT);
        const url = `${opts.serverUrl}/api/inspect/${opts.layer}/${encodeURIComponent(opts.timestamp)}/${lat}/${lon}`;
        const resp = await fetchWithTimeout(url, {}, opts.signal);
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
        recordSpanError(span, err);
      }
      return { ok: false, value: null, unit: "", source: "unavailable", reason: "no_source" };
    },
    {
      "inspector.layer": opts.layer,
      "inspector.timestamp": opts.timestamp,
    },
  );
}

const RADAR_FAMILY = new Set<LayerType>(["radar", "radar-composite", "radar-hrrr"]);

/**
 * Where a frame's value lives. The radar timeline mixes observed MRMS,
 * nowcast and HRRR frames under one layer; each has its own grids.
 */
export function inspectSourceFor(layer: LayerType, frameSource: string | null | undefined): "nowcast" | LayerType {
  if (!RADAR_FAMILY.has(layer)) return layer;
  if (frameSource === "nowcast") return "nowcast";
  if (frameSource === "radar-hrrr") return "radar-hrrr";
  return layer;
}

/**
 * The value at a point for one timeline frame. Nowcast grids are stored per
 * run, so they're read from the point series /api/nowcast returns rather
 * than /api/inspect.
 */
export async function inspectFrame(
  opts: InspectOptions & { frameSource?: string | null },
): Promise<InspectReading> {
  const source = inspectSourceFor(opts.layer, opts.frameSource);
  if (source !== "nowcast") return inspectPoint({ ...opts, layer: source });
  try {
    const series = await fetchRadarNowcast(opts.serverUrl, opts.lat, opts.lon, opts.signal);
    const at = Date.parse(opts.timestamp);
    const point = series.points.find((p) => Date.parse(p.timestamp) === at);
    if (point && point.dbz != null) return { ok: true, value: point.dbz, unit: "dBZ", source: "grid" };
    return { ok: false, value: null, unit: "dBZ", source: "unavailable", reason: series.reason ?? "no_point" };
  } catch {
    return { ok: false, value: null, unit: "dBZ", source: "unavailable", reason: "no_source" };
  }
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
