/**
 * Legend stops for map layers, built from the tile renderer's own tables (`/api/palettes`), so the
 * legend shows exactly the colors on the map.
 */
import type { LayerType, Palette } from "../types/weather";
import { LAYER_PALETTE_KEY, type ColorRange, type ServerPalettes } from "../../../web/src/shared/serverPalettes";

export type { ColorRange, ColorTable, ServerPalettes } from "../../../web/src/shared/serverPalettes";

const LAYER_TABLES: Partial<Record<LayerType, { title: string; unit: string; whole?: boolean }>> = {
  temperature: { title: "Temperature", unit: "°F" },
  "feels-like": { title: "Feels Like", unit: "°F" },
  dewpoint: { title: "Dew Point", unit: "°F" },
  wind: { title: "Wind", unit: "mph" },
  "precip-accum": { title: "Rain Total", unit: "in" },
  "precip-24h": { title: "Past 24 Hr", unit: "in" },
  cloud: { title: "Clouds", unit: "%" },
  // Band edges sit halfway between whole index values (0.5, 2.5, ...); label the whole values.
  uv: { title: "UV Index", unit: "WHO", whole: true },
};

export interface ServerLegend {
  title: string;
  /** Top to bottom: highest value first, like the other app legends. */
  stops: { label: string; color: string }[];
}

const hex = ([r, g, b]: ColorRange["rgba"]) =>
  "#" + [r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("");
const fmt = (v: number) => String(Number(v.toFixed(2)));

export interface PrecipTypeRamp {
  label: "Rain" | "Snow" | "Mix";
  /** Weak to strong. */
  colors: string[];
}

/** Rain, snow and mix ramps of the radar palette, for frames the server colors by precipitation type. */
export function precipTypeRamps(palettes: ServerPalettes | null | undefined, palette: Palette): PrecipTypeRamp[] | null {
  const table = (palettes?.[palette] ?? palettes?.classic)?.reflectivity;
  const snow = table?.types?.snow?.ranges;
  const mix = table?.types?.mix?.ranges;
  if (!table?.ranges?.length || !snow?.length || !mix?.length) return null;
  const colors = (ranges: ColorRange[]) => ranges.map((r) => hex(r.rgba));
  return [
    { label: "Rain", colors: colors(table.ranges) },
    { label: "Snow", colors: colors(snow) },
    { label: "Mix", colors: colors(mix) },
  ];
}

export function serverLegend(
  palettes: ServerPalettes | null | undefined,
  palette: Palette,
  layer: LayerType,
  maxStops = 7,
): ServerLegend | null {
  const spec = LAYER_TABLES[layer];
  const ranges = spec && (palettes?.[palette] ?? palettes?.classic)?.[LAYER_PALETTE_KEY[layer]]?.ranges;
  if (!spec || !ranges || ranges.length < 2) return null;
  // Every band when it fits; otherwise an even sample that keeps the first and last band.
  const n = Math.min(maxStops, ranges.length);
  const picked = Array.from({ length: n }, (_, i) => ranges[Math.round((i * (ranges.length - 1)) / (n - 1))]);
  const label = (v: number) => fmt(spec.whole ? Math.ceil(v) : v);
  const stops = picked.map((r, i) => ({ label: label(r.min) + (i === n - 1 ? "+" : ""), color: hex(r.rgba) }));
  return { title: `${spec.title} (${spec.unit})`, stops: stops.reverse() };
}
