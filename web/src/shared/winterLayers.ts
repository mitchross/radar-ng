/**
 * The "Winter" layers (NOAA NBM snowfall and ice totals, SNODAS snow depth, NOHRSC 24 h snowfall, WPC
 * snow chance): ids, names, one-line hints, credit and point readouts, declared once for the web radar
 * and the app. Add a row to WINTER_LAYERS to add a layer to both pickers.
 */

export type WinterLayerId =
  | "snowfall-forecast"
  | "ice-forecast"
  | "snow-depth"
  | "snowfall-24h"
  | "snow-chance-1"
  | "snow-chance-4"
  | "snow-chance-8";

export interface WinterLayerInfo {
  id: WinterLayerId;
  name: string;
  /** One plain-English line: what the colors mean. */
  hint: string;
  /** Unit the server renders and the inspector reports. */
  unit: string;
  /** Server palette key (rust/crates/radar-tiler/assets/*.json). */
  paletteKey: string;
  /** Who made the data, shown under the legend. */
  credit: string;
  /** Values below this read as "none" in the inspector. */
  floor: number;
  /** Inspector decimals. */
  digits: number;
  /** Inspector text below the floor. */
  none: string;
  /** A daily or twice-daily analysis with one frame; the legend shows its date. */
  observed?: boolean;
  /** Layers sharing a row show as one picker entry with a chip per layer (snow chance 1 / 4 / 8 in). */
  row?: string;
  /** Chip label inside the row. */
  chip?: string;
}

const snowChance = (inches: 1 | 4 | 8): WinterLayerInfo => ({
  id: `snow-chance-${inches}`,
  name: "Snow chance",
  hint: `Chance of at least ${inches} in of snow in the 24 hours ending at each frame's time`,
  unit: "%",
  paletteKey: "snow_chance",
  credit: "NOAA WPC",
  floor: 5,
  digits: 0,
  none: "Under 5%",
  row: "snow-chance",
  chip: `${inches} in`,
});

export const WINTER_LAYERS: readonly WinterLayerInfo[] = [
  {
    id: "snowfall-forecast",
    name: "Snow total",
    hint: "Snow still to fall from now to each frame's time",
    unit: "in",
    paletteKey: "snowfall",
    credit: "NOAA NBM",
    floor: 0.1,
    digits: 1,
    none: "No snow",
  },
  {
    id: "ice-forecast",
    name: "Ice total",
    hint: "Ice building up on trees and lines from now to each frame's time",
    unit: "in",
    paletteKey: "ice_accum",
    credit: "NOAA NBM",
    floor: 0.01,
    digits: 2,
    none: "No ice",
  },
  {
    id: "snow-depth",
    name: "Snow depth",
    hint: "Snow on the ground, updated once a day",
    unit: "in",
    paletteKey: "snow_depth",
    credit: "NOAA NOHRSC SNODAS",
    floor: 1,
    digits: 0,
    none: "No snow on the ground",
    observed: true,
  },
  {
    id: "snowfall-24h",
    name: "Past 24 h",
    hint: "Snow that fell in the 24 hours before the analysis time",
    unit: "in",
    paletteKey: "snowfall",
    credit: "NOAA NOHRSC",
    floor: 0.1,
    digits: 1,
    none: "No new snow",
    observed: true,
  },
  snowChance(1),
  snowChance(4),
  snowChance(8),
];

/** Sheet hint while the server has no winter frames yet (feature off, or first run still ingesting). */
export const WINTER_EMPTY_HINT = "No winter layers on this server yet. They appear after the next NOAA update.";

/** Sheet hint when the selected layer is not a winter layer. */
export const WINTER_DEFAULT_HINT =
  "Snow and ice: forecast totals out to 3 days, snow on the ground, past 24 h snowfall, snow chance";

export function winterLayer(id: string): WinterLayerInfo | undefined {
  return WINTER_LAYERS.find((l) => l.id === id);
}

export const isWinterLayer = (id: string): boolean => winterLayer(id) !== undefined;

/** Point readout for a winter layer, or null for any other layer. Values are in the layer's unit. */
export function formatWinterReading(layer: string, value: number): string | null {
  const info = winterLayer(layer);
  if (!info) return null;
  if (value < info.floor) return info.none;
  return info.unit === "%" ? `${value.toFixed(info.digits)}%` : `${value.toFixed(info.digits)} ${info.unit}`;
}

/**
 * Picker entries from the available winter layers: one per layer, except a row (snow chance) folds into
 * one entry. That entry is the active layer when it is in the row, else the row's first available layer.
 */
export function winterPickerRows<T extends { id: string }>(available: readonly T[], activeLayer: string): T[] {
  const rows: T[] = [];
  const seen = new Set<string>();
  for (const l of available) {
    const row = winterLayer(l.id)?.row;
    if (!row) {
      rows.push(l);
      continue;
    }
    if (seen.has(row)) continue;
    seen.add(row);
    rows.push(available.find((o) => o.id === activeLayer && winterLayer(o.id)?.row === row) ?? l);
  }
  return rows;
}

/** The chips under the picker while a layer in a row is active (snow chance 1 / 4 / 8 in), else none. */
export function winterRowChips<T extends { id: string }>(available: readonly T[], activeLayer: string): T[] {
  const row = winterLayer(activeLayer)?.row;
  return row ? available.filter((l) => winterLayer(l.id)?.row === row) : [];
}

/** The winter sheet's hint line: the selected layer's hint and credit, or the group summary. */
export function winterSheetHint(activeLayer: string): string {
  const info = winterLayer(activeLayer);
  return info ? `${info.hint} · ${info.credit}` : `${WINTER_DEFAULT_HINT} · NOAA`;
}

/**
 * "As of Nov 17" for an observed layer's frame (ISO timestamp), or null for forecast layers. The frame
 * stays up when the next file is late, so the date tells the reader how old it is.
 */
export function winterAsOf(layer: string, timestamp: string | undefined, timeZone?: string): string | null {
  if (!winterLayer(layer)?.observed || !timestamp) return null;
  const t = new Date(timestamp);
  if (Number.isNaN(t.getTime())) return null;
  return `As of ${t.toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone })}`;
}

/** Map layer id to palette key, merged into the shared fallback map. */
export const WINTER_PALETTE_KEYS: Readonly<Record<string, string>> = Object.fromEntries(
  WINTER_LAYERS.map((l) => [l.id, l.paletteKey]),
);
