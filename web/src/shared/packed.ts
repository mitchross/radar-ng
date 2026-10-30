/**
 * Packed data tiles (docs/reference/packed-tiles.md): the manifest types, the strict metadata parser and
 * the pure helpers the web radar and the app both use. Nothing here touches the DOM or GL.
 *
 * A packed tile is an RGB PNG atlas of quantized frames (3 slots per pixel). The overlay colors a
 * slot through a 256 x 1 lookup table, so playback changes a uniform instead of fetching tiles.
 */
import type { ColorRange, ColorTable } from "./serverPalettes";

export const PACKED_TILE_SIZE = 512;

export interface PackedBucket {
  id: string;
  /** Bucket version; it goes into the tile URL so every update is a new URL. */
  v: number;
  /** Slot timestamps, the same strings as the layer's `frames[].timestamp`. */
  slots: string[];
}

/** `layers.<layer>.packed` in the manifest. */
export interface PackedMeta {
  version: number;
  tile_size: number;
  min_zoom: number;
  max_zoom: number;
  cols: number;
  max_slots: number;
  color_key: string;
  scale: number;
  offset: number;
  url_template: string;
  buckets: PackedBucket[];
  /** v2 only: code = class * stride + q, classes such as ["rain", "snow", "mix"]. */
  types?: PackedTypes;
}

export interface PackedTypes {
  stride: number;
  classes: string[];
}

const DEFAULT_NO_DATA_BELOW = -999;

const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
const posInt = (v: unknown): v is number => Number.isInteger(v) && (v as number) > 0;

/**
 * The layer's packed metadata when this client can draw it, else null (raster fallback).
 * Lenient on purpose: bad packed metadata must not fail the whole manifest. The server never sets
 * `categorical: true` today; the spec reserves it for id layers, which stay on rasters.
 */
export function parsePackedMeta(value: unknown): PackedMeta | null {
  if (typeof value !== "object" || value === null) return null;
  const m = value as Record<string, unknown>;
  if (m.tile_size !== PACKED_TILE_SIZE || m.categorical === true) return null;
  // v2 adds typed codes; any other version, or types without v2, stays on rasters.
  const types = parseTypes(m.types);
  if (!(m.version === 1 && m.types === undefined) && !(m.version === 2 && types)) return null;
  if (!posInt(m.cols) || !posInt(m.max_slots)) return null;
  if (!Number.isInteger(m.min_zoom) || !Number.isInteger(m.max_zoom)) return null;
  if ((m.min_zoom as number) < 0 || (m.max_zoom as number) < (m.min_zoom as number)) return null;
  if (!finite(m.scale) || m.scale <= 0 || !finite(m.offset)) return null;
  if (typeof m.color_key !== "string" || typeof m.url_template !== "string") return null;
  if (!Array.isArray(m.buckets)) return null;
  const buckets: PackedBucket[] = [];
  for (const b of m.buckets as unknown[]) {
    const r = b as Record<string, unknown> | null;
    if (!r || typeof r.id !== "string" || !finite(r.v) || !Array.isArray(r.slots)) return null;
    if (r.slots.length > (m.max_slots as number) || r.slots.some((s) => typeof s !== "string")) return null;
    buckets.push({ id: r.id, v: r.v, slots: r.slots as string[] });
  }
  return { ...(m as unknown as PackedMeta), buckets, types: types ?? undefined };
}

function parseTypes(value: unknown): PackedTypes | null {
  if (typeof value !== "object" || value === null) return null;
  const t = value as Record<string, unknown>;
  if (!posInt(t.stride) || t.stride > 256 || !Array.isArray(t.classes) || t.classes.length === 0) return null;
  if (t.classes.some((c) => typeof c !== "string")) return null;
  return { stride: t.stride, classes: t.classes as string[] };
}

/** The ranges a precipitation class draws with: `ranges` for the first class (rain), else its ramp. */
export function classRanges(table: ColorTable, types: PackedTypes | undefined, cls: number): ColorRange[] {
  const rain = table.ranges ?? [];
  if (!types || cls === 0) return rain;
  return table.types?.[types.classes[cls]]?.ranges ?? rain;
}

/**
 * 256 x 1 RGBA table, unpremultiplied: entry q colors `value = offset + q * scale`. With `types`,
 * entry `class * stride + q` colors q with that class's ramp.
 * q = 0, values below `no_data_below` (rain ramp only, as on the server) and values in no range are
 * transparent. Ranges are half-open [min, max); a later overlapping range wins (the renderer's rules).
 */
export function buildLut(
  table: ColorTable | null | undefined,
  scale: number,
  offset: number,
  types?: PackedTypes,
): Uint8Array {
  const lut = new Uint8Array(256 * 4);
  if (!table?.ranges || table.ranges.length === 0) return lut;
  const noDataBelow = finite(table.no_data_below) ? table.no_data_below : DEFAULT_NO_DATA_BELOW;
  const stride = types ? types.stride : 256;
  for (let code = 1; code < 256; code++) {
    const cls = Math.floor(code / stride);
    const q = code % stride;
    if (q === 0 || (types && cls >= types.classes.length)) continue;
    // Round off float noise (0.05 * 20 = 1.0000000000000002) so band edges compare exactly.
    const value = Math.round((offset + q * scale) * 1e6) / 1e6;
    if (cls === 0 && value < noDataBelow) continue;
    let rgba: ColorRange["rgba"] | null = null;
    for (const r of classRanges(table, types, cls)) {
      if (value >= r.min && value < r.max) rgba = r.rgba;
    }
    if (!rgba) continue;
    for (let c = 0; c < 4; c++) lut[code * 4 + c] = Math.max(0, Math.min(255, Math.trunc(rgba[c] ?? 0)));
  }
  return lut;
}

export interface SlotAddress {
  block: number;
  /** 0 = R, 1 = G, 2 = B. */
  channel: number;
  /** Atlas pixel origin of the slot's block. */
  x0: number;
  y0: number;
}

export function slotAddress(slot: number, cols = 2, tile = PACKED_TILE_SIZE): SlotAddress {
  const block = Math.floor(slot / 3);
  return {
    block,
    channel: slot % 3,
    x0: (block % cols) * tile,
    y0: Math.floor(block / cols) * tile,
  };
}

export interface PackedLocation {
  bucket: string;
  v: number;
  slot: number;
  /** The bucket's slot count in this manifest; a tile fetched for it holds at least these slots. */
  slots: number;
}

/** The bucket and slot that hold the frame with this timestamp, or null (raster path). */
export function locateFrame(meta: Pick<PackedMeta, "buckets" | "max_slots"> | null | undefined, timestamp: string): PackedLocation | null {
  if (!meta) return null;
  const epoch = Date.parse(timestamp);
  let byEpoch: PackedLocation | null = null;
  for (const b of meta.buckets) {
    for (let slot = 0; slot < b.slots.length && slot < meta.max_slots; slot++) {
      if (b.slots[slot] === timestamp) return { bucket: b.id, v: b.v, slot, slots: b.slots.length };
      // Same instant written with another offset spelling ("Z" vs "+00:00").
      if (!byEpoch && Number.isFinite(epoch) && Date.parse(b.slots[slot]) === epoch) {
        byEpoch = { bucket: b.id, v: b.v, slot, slots: b.slots.length };
      }
    }
  }
  return byEpoch;
}
