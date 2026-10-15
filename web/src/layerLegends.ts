/**
 * Legends for the non-radar map layers, built from the tile renderer's own tables (`/api/palettes`),
 * so the legend shows exactly the colors on the map.
 */
import type { PaletteId } from "./palettes";

export interface ColorRange {
  min: number;
  max: number;
  rgba: [number, number, number, number];
}
export interface ColorTable {
  unit?: string;
  ranges: ColorRange[];
}
export type ServerPalettes = Partial<Record<PaletteId, Record<string, ColorTable>>>;

/** Map layer id to palette key, as the server's HRRR `layer_color_key` and MRMS QPE map them. */
const LAYER_TABLES: Record<string, { key: string; title: string; unit: string }> = {
  temperature: { key: "temperature", title: "Temperature", unit: "°F" },
  "feels-like": { key: "temperature", title: "Feels like", unit: "°F" },
  dewpoint: { key: "dewpoint", title: "Dew point", unit: "°F" },
  wind: { key: "wind_speed", title: "Wind", unit: "mph" },
  "precip-accum": { key: "precip_accum", title: "Rain total", unit: "in" },
  "precip-24h": { key: "precip_accum", title: "Past 24 hr precip", unit: "in" },
  cloud: { key: "cloud_cover", title: "Clouds", unit: "%" },
};

export interface LayerLegend {
  title: string;
  gradient: string;
  labels: [string, string];
}

const fmt = (v: number) => String(Number(v.toFixed(2)));

export function layerLegend(palettes: ServerPalettes | null, palette: PaletteId, layer: string): LayerLegend | null {
  const spec = LAYER_TABLES[layer];
  const ranges = spec && (palettes?.[palette] ?? palettes?.classic)?.[spec.key]?.ranges;
  if (!spec || !ranges?.length) return null;
  const lo = ranges[0].min;
  const hi = ranges[ranges.length - 1].max;
  const pos = (v: number) => `${(((v - lo) / (hi - lo)) * 100).toFixed(1)}%`;
  const color = ([r, g, b]: ColorRange["rgba"]) => `rgb(${r},${g},${b})`;
  const stops = ranges.map((r) => `${color(r.rgba)} ${pos(r.min)} ${pos(Math.min(r.max, hi))}`).join(",");
  return {
    title: `${spec.title} (${spec.unit})`,
    gradient: `linear-gradient(90deg,${stops})`,
    labels: [fmt(lo), fmt(ranges[ranges.length - 1].min) + "+"],
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
