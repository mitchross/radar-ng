/**
 * The `/api/palettes` response and the layer-to-palette-key map, declared once for the web and the app.
 * Body: `{"<palette>": {"<color key>": ColorTable}}` (rust/crates/radar-api/src/palettes.rs). Pure types
 * and one table, so both clients share it.
 */

export type ServerPaletteId = "classic" | "muted" | "vivid";

export interface ColorRange {
  min: number;
  max: number;
  rgba: [number, number, number, number];
}

/** One color key. Every key the clients read has `ranges`; `precip_type` is categorical and unread. */
export interface ColorTable {
  unit?: string;
  ranges: ColorRange[];
  no_data_below?: number;
  /** Reflectivity only: ramps of the other precipitation classes (snow, mix); `ranges` is rain. */
  types?: Record<string, { ranges: ColorRange[] }>;
}

export type ServerPalettes = Partial<Record<ServerPaletteId, Record<string, ColorTable>>>;

/**
 * Map layer id to palette key, as the server's HRRR `layer_color_key` and MRMS QPE map them
 * (rust/crates/radar-ingest/src/hrrr/state.rs). A layer missing here has no server legend.
 * Fallback only: current servers send each layer's key as `palette_key` in the manifest
 * (see {@link layerPaletteKey}); this map serves older servers.
 */
export const LAYER_PALETTE_KEY: Readonly<Record<string, string>> = {
  temperature: "temperature",
  "feels-like": "temperature",
  dewpoint: "dewpoint",
  wind: "wind_speed",
  "precip-accum": "precip_accum",
  "precip-24h": "precip_accum",
  cloud: "cloud_cover",
  uv: "uv_index",
};

/** The palette key of a layer: the server's manifest `palette_key` when it sends one, else the local map. */
export function layerPaletteKey(layer: string, serverKey?: string | null): string | undefined {
  return serverKey || LAYER_PALETTE_KEY[layer];
}
