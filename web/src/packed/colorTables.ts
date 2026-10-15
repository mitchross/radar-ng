/**
 * The tile renderer's color tables, from `/api/palettes` ({palette: {color_key: table}}).
 * Until (or unless) that answers, reflectivity falls back to the bands in palettes.ts.
 */
import { PALETTES, type PaletteId } from "../palettes";
import type { ColorTable } from "./format";

export type ServerPalettes = Partial<Record<PaletteId, Record<string, ColorTable>>>;

/** palettes.ts mirrors the reflectivity JSON: the top band ends at 75 dBZ and nothing below 5 dBZ draws. */
const REFLECTIVITY_TOP = 75;
const REFLECTIVITY_NO_DATA_BELOW = 5;

export function fallbackTable(palette: PaletteId, colorKey: string): ColorTable | undefined {
  if (colorKey !== "reflectivity") return undefined;
  const bands = PALETTES[palette]?.bands;
  if (!bands) return undefined;
  return {
    unit: "dBZ",
    ranges: bands.map((b, i) => ({ min: b.min, max: bands[i + 1]?.min ?? REFLECTIVITY_TOP, rgba: b.rgba })),
    no_data_below: REFLECTIVITY_NO_DATA_BELOW,
  };
}

export function colorTable(server: ServerPalettes | null, palette: PaletteId, colorKey: string): ColorTable | undefined {
  return server?.[palette]?.[colorKey] ?? fallbackTable(palette, colorKey);
}

export async function fetchServerPalettes(): Promise<ServerPalettes | null> {
  try {
    const res = await fetch("/api/palettes");
    return res.ok ? ((await res.json()) as ServerPalettes) : null;
  } catch {
    return null;
  }
}
