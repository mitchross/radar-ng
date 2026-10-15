/**
 * The tile renderer's color tables, from `/api/palettes` ({palette: {color_key: table}}).
 * Until (or unless) that answers, reflectivity falls back to the bands in palettes.ts.
 */
import type { ServerPalettes } from "../layerLegends";
import { PALETTES, type Band, type PaletteId } from "../palettes";
import type { ColorTable } from "./format";

export type { ServerPalettes };

/** palettes.ts mirrors the reflectivity JSON: the top band ends at 75 dBZ and nothing below 5 dBZ draws. */
const REFLECTIVITY_TOP = 75;
const REFLECTIVITY_NO_DATA_BELOW = 5;

export function fallbackTable(palette: PaletteId, colorKey: string): ColorTable | undefined {
  if (colorKey !== "reflectivity") return undefined;
  const p = PALETTES[palette];
  if (!p) return undefined;
  const ranges = (bands: Band[]) => bands.map((b, i) => ({ min: b.min, max: bands[i + 1]?.min ?? REFLECTIVITY_TOP, rgba: b.rgba }));
  return {
    unit: "dBZ",
    ranges: ranges(p.bands),
    no_data_below: REFLECTIVITY_NO_DATA_BELOW,
    types: { snow: { ranges: ranges(p.snow) }, mix: { ranges: ranges(p.mix) } },
  };
}

export function colorTable(server: ServerPalettes | null, palette: PaletteId, colorKey: string): ColorTable | undefined {
  return server?.[palette]?.[colorKey] ?? fallbackTable(palette, colorKey);
}
