/**
 * The "Winter" layers (NOAA NBM snowfall and ice totals; later snow depth, 24 h snowfall, snow chance):
 * ids, names, one-line hints, credit and point readouts, declared once for the web radar and the app.
 * Add a row to WINTER_LAYERS to add a layer to both pickers.
 */

export type WinterLayerId = "snowfall-forecast" | "ice-forecast";

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
}

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
];

/** Sheet hint while the server has no winter frames yet (feature off, or first run still ingesting). */
export const WINTER_EMPTY_HINT = "No snow or ice forecast on this server yet. It appears after the next NBM run.";

export function winterLayer(id: string): WinterLayerInfo | undefined {
  return WINTER_LAYERS.find((l) => l.id === id);
}

export const isWinterLayer = (id: string): boolean => winterLayer(id) !== undefined;

/** Point readout for a winter layer, or null for any other layer. Values are in the layer's unit. */
export function formatWinterReading(layer: string, value: number): string | null {
  const info = winterLayer(layer);
  if (!info) return null;
  return value < info.floor ? info.none : `${value.toFixed(info.digits)} ${info.unit}`;
}

/** Map layer id to palette key, merged into the shared fallback map. */
export const WINTER_PALETTE_KEYS: Readonly<Record<string, string>> = Object.fromEntries(
  WINTER_LAYERS.map((l) => [l.id, l.paletteKey]),
);
