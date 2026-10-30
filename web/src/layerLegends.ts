/**
 * Legends for the non-radar map layers, built from the tile renderer's own tables (`/api/palettes`),
 * so the legend shows exactly the colors on the map.
 */
import type { PaletteId } from "./palettes";
import { LAYER_PALETTE_KEY, type ColorRange, type ServerPalettes } from "./shared/serverPalettes";

const LAYER_TABLES: Record<string, { title: string; unit: string; whole?: boolean }> = {
  temperature: { title: "Temperature", unit: "°F" },
  "feels-like": { title: "Feels like", unit: "°F" },
  dewpoint: { title: "Dew point", unit: "°F" },
  wind: { title: "Wind", unit: "mph" },
  "precip-accum": { title: "Rain total", unit: "in" },
  "precip-24h": { title: "Past 24 hr precip", unit: "in" },
  cloud: { title: "Clouds", unit: "%" },
  // Band edges sit halfway between whole index values (0.5, 2.5, ...); label the whole values.
  uv: { title: "UV index", unit: "WHO", whole: true },
};

export interface LayerLegend {
  title: string;
  gradient: string;
  labels: [string, string];
}

const fmt = (v: number) => String(Number(v.toFixed(2)));

export function layerLegend(palettes: ServerPalettes | null, palette: PaletteId, layer: string): LayerLegend | null {
  const spec = LAYER_TABLES[layer];
  const ranges = spec && (palettes?.[palette] ?? palettes?.classic)?.[LAYER_PALETTE_KEY[layer]]?.ranges;
  if (!spec || !ranges?.length) return null;
  const label = (v: number) => fmt(spec.whole ? Math.ceil(v) : v);
  const lo = ranges[0].min;
  const hi = ranges[ranges.length - 1].max;
  const pos = (v: number) => `${(((v - lo) / (hi - lo)) * 100).toFixed(1)}%`;
  const color = ([r, g, b]: ColorRange["rgba"]) => `rgb(${r},${g},${b})`;
  const stops = ranges.map((r) => `${color(r.rgba)} ${pos(r.min)} ${pos(Math.min(r.max, hi))}`).join(",");
  return {
    title: `${spec.title} (${spec.unit})`,
    gradient: `linear-gradient(90deg,${stops})`,
    labels: [label(lo), label(ranges[ranges.length - 1].min) + "+"],
  };
}

export async function fetchServerPalettes(): Promise<ServerPalettes | null> {
  try {
    const res = await fetch("/api/palettes");
    return res.ok ? ((await res.json()) as ServerPalettes) : null;
  } catch {
    return null;
  }
}

/** WHO category of a UV Index reading, on the rounded index as forecasts report it. */
export function uvCategory(uv: number): string {
  const i = Math.round(uv);
  return i <= 2 ? "Low" : i <= 5 ? "Moderate" : i <= 7 ? "High" : i <= 10 ? "Very high" : "Extreme";
}
