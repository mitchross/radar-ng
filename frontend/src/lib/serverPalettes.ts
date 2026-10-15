/**
 * Legend stops for map layers, built from the tile renderer's own tables (`/api/palettes`), so the
 * legend shows exactly the colors on the map.
 */
import type { LayerType, Palette } from "../types/weather";

export interface ColorRange {
  min: number;
  max: number;
  rgba: [number, number, number, number];
}
export interface ColorTable {
  unit?: string;
  ranges: ColorRange[];
}
export type ServerPalettes = Partial<Record<Palette, Record<string, ColorTable>>>;

/** Layer to palette key, as the server's HRRR `layer_color_key` maps them. */
const LAYER_TABLES: Partial<Record<LayerType, { key: string; title: string; unit: string }>> = {
  temperature: { key: "temperature", title: "Temperature", unit: "°F" },
  wind: { key: "wind_speed", title: "Wind", unit: "mph" },
  "precip-accum": { key: "precip_accum", title: "Rain Total", unit: "in" },
  cloud: { key: "cloud_cover", title: "Clouds", unit: "%" },
};

export interface ServerLegend {
  title: string;
  /** Top to bottom: highest value first, like the other app legends. */
  stops: { label: string; color: string }[];
}

const hex = ([r, g, b]: ColorRange["rgba"]) =>
  "#" + [r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("");
const fmt = (v: number) => String(Number(v.toFixed(2)));

export function serverLegend(
  palettes: ServerPalettes | null | undefined,
  palette: Palette,
  layer: LayerType,
  maxStops = 7,
): ServerLegend | null {
  const spec = LAYER_TABLES[layer];
  const ranges = spec && (palettes?.[palette] ?? palettes?.classic)?.[spec.key]?.ranges;
  if (!spec || !ranges || ranges.length < 2) return null;
  // Every band when it fits; otherwise an even sample that keeps the first and last band.
  const n = Math.min(maxStops, ranges.length);
  const picked = Array.from({ length: n }, (_, i) => ranges[Math.round((i * (ranges.length - 1)) / (n - 1))]);
  const stops = picked.map((r, i) => ({ label: fmt(r.min) + (i === n - 1 ? "+" : ""), color: hex(r.rgba) }));
  return { title: `${spec.title} (${spec.unit})`, stops: stops.reverse() };
}
