/**
 * "Temp vs normal": the map layer (forecast daily high minus the 1991-2020 normal high, degrees F) and
 * the current-conditions line ("High 58°, 12° above normal") from `GET /api/normals/{lat}/{lon}`.
 * Declared once for the web radar and the app. Both hide outside the lower 48 and until the server
 * has built the normals (503) or has frames for the layer.
 */
import type { ColorRange } from "./serverPalettes";

export const TEMP_NORMAL_LAYER = {
  id: "temp-anomaly",
  name: "Temp vs Normal",
  /** Server palette key (rust/crates/radar-tiler/assets/*.json). */
  paletteKey: "temp_anomaly",
  hint: "Each day's forecast high against the 1991-2020 normal high, days 1 to 7",
  credit: "GFS forecast high vs NOAA NCEI 1991-2020 normals.",
} as const;

/** Rough lower-48 box. The normals grid is lower 48 only; outside it the server answers 404. */
export function inLower48(lat: number, lon: number): boolean {
  return lat >= 24.5 && lat <= 49.5 && lon >= -125 && lon <= -66.5;
}

/** The point API path. `date` is the local calendar date, `YYYY-MM-DD`. */
export function normalsPath(lat: number, lon: number, date: string): string {
  return `/api/normals/${lat.toFixed(2)}/${lon.toFixed(2)}?date=${date}`;
}

export interface DayNormals {
  date: string;
  highF: number;
  lowF: number;
}

/** Parse the API body; anything unexpected is `null` (the line hides). */
export function parseNormals(body: unknown): DayNormals | null {
  if (!body || typeof body !== "object") return null;
  const b = body as Record<string, unknown>;
  const num = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : null);
  const highF = num(b.high_f);
  const lowF = num(b.low_f);
  if (highF === null || lowF === null || typeof b.date !== "string") return null;
  return { date: b.date, highF, lowF };
}

/** Signed whole degrees in words: "12° above normal", "3° below normal", "near normal". */
export function anomalyWords(deltaF: number): string {
  const d = Math.round(deltaF);
  if (d === 0) return "near normal";
  return `${Math.abs(d)}\u00B0 ${d > 0 ? "above" : "below"} normal`;
}

/**
 * The current-conditions line, or `null` when there is no forecast high or no normal. High compared
 * with the normal high, never the daily mean. `celsius` words both numbers in degrees C.
 */
export function normalsLine(forecastHighF: number | null | undefined, normals: DayNormals | null, celsius = false): string | null {
  if (normals === null || typeof forecastHighF !== "number" || !Number.isFinite(forecastHighF)) return null;
  const k = celsius ? 5 / 9 : 1;
  const high = celsius ? (forecastHighF - 32) * k : forecastHighF;
  return `High ${Math.round(high)}\u00B0, ${anomalyWords((forecastHighF - normals.highF) * k)}`;
}

/** Inspector text for a map value: "12° above normal". */
export function formatAnomalyReading(value: number | null | undefined): string | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return anomalyWords(value);
}

export interface AnomalyLegendStop {
  label: string;
  color: string;
}

/** Classic palette colors, used until the server palettes load (-20 to +20 by 5). */
const FALLBACK: AnomalyLegendStop[] = [
  { label: "-20\u00B0", color: "#4575b4" },
  { label: "-10\u00B0", color: "#abd9e9" },
  { label: "Normal", color: "#f5f5f5" },
  { label: "+10\u00B0", color: "#f46d43" },
  { label: "+20\u00B0", color: "#a50026" },
];

const hex = ([r, g, b]: ColorRange["rgba"]) => "#" + [r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("");

/** Five legend steps, coldest first, colored from the server's `temp_anomaly` ranges when given. */
export function tempAnomalyLegend(ranges?: readonly ColorRange[] | null): AnomalyLegendStop[] {
  if (!ranges?.length) return FALLBACK;
  return [-20, -10, 0, 10, 20].map((v, i) => {
    const range = ranges.find((r) => v >= r.min && v < r.max);
    return { label: FALLBACK[i].label, color: range ? hex(range.rgba) : FALLBACK[i].color };
  });
}
