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
  /** v2 only: code = class * stride + q, classes such as ["rain", "snow", "mix", "ice"]. */
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

/**
 * Where the lowest band of a ramp starts when it fades in from the floor: this fraction of its alpha.
 * Only tables whose `no_data_below` is the ramp's own floor (reflectivity, wind, cloud, rain total,
 * AQ: the low end means "nothing") fade; full-coverage fields such as temperature (-999) do not.
 */
export const SMOOTH_FLOOR_ALPHA = 0.5;

type Rgba = ColorRange["rgba"];

interface Segment {
  lo: number;
  hi: number;
  /** null: transparent (no range, or below `no_data_below`). */
  rgba: Rgba | null;
}

/**
 * The stepped ramp as segments of constant colour, by the renderer's rules (half-open ranges, the
 * later overlapping range wins, values below `noDataBelow` transparent). Equal neighbours merge.
 */
function rampSegments(ranges: ColorRange[], noDataBelow: number): Segment[] {
  const edges = new Set<number>();
  for (const r of ranges) {
    edges.add(r.min);
    edges.add(r.max);
  }
  if (Number.isFinite(noDataBelow)) edges.add(noDataBelow);
  const sorted = [...edges].filter(Number.isFinite).sort((a, b) => a - b);
  const out: Segment[] = [];
  for (let i = 0; i + 1 < sorted.length; i++) {
    const lo = sorted[i];
    let rgba: Rgba | null = null;
    if (lo >= noDataBelow) for (const r of ranges) if (lo >= r.min && lo < r.max) rgba = r.rgba;
    const prev = out[out.length - 1];
    const same = prev && (prev.rgba === rgba || (prev.rgba && rgba && prev.rgba.every((c, k) => c === rgba![k])));
    if (same) prev.hi = sorted[i + 1];
    else out.push({ lo, hi: sorted[i + 1], rgba });
  }
  return out;
}

/**
 * Colour of `value` on the smooth ramp: each band's colour sits at the band's start and blends
 * linearly into the next band's colour at its start, so every band edge keeps its stepped colour.
 * The top band, and a band followed by a gap, stay flat. With `fadeFloor`, the lowest band of the
 * ramp starts at SMOOTH_FLOOR_ALPHA of its alpha and reaches the next stop's. Transparent wherever
 * the stepped ramp is. Unrounded floats, 0..255.
 */
function smoothRampColor(segments: readonly Segment[], value: number, fadeFloor: boolean): Rgba | null {
  for (let i = 0; i < segments.length; i++) {
    const s = segments[i];
    if (value < s.lo || value >= s.hi) continue;
    if (!s.rgba) return null;
    const next = segments[i + 1];
    const floor = fadeFloor && !segments[i - 1]?.rgba;
    const start: Rgba = floor ? [s.rgba[0], s.rgba[1], s.rgba[2], s.rgba[3] * SMOOTH_FLOOR_ALPHA] : s.rgba;
    const end = next?.rgba && next.lo === s.hi ? next.rgba : floor ? s.rgba : null;
    if (!end) return start;
    const t = (value - s.lo) / (s.hi - s.lo);
    return [0, 1, 2, 3].map((k) => start[k] + (end[k] - start[k]) * t) as Rgba;
  }
  return null;
}

/**
 * The smooth twin of buildLut: the same 256 x 1 RGBA layout and the same transparent entries, but
 * entry q holds the smooth ramp colour of `offset + q * scale` (see smoothRampColor). With `types`,
 * each class gets its own ramp; a client blending between entries must stay inside one class.
 */
export function buildSmoothLut(
  table: ColorTable | null | undefined,
  scale: number,
  offset: number,
  types?: PackedTypes,
): Uint8Array {
  const lut = new Uint8Array(256 * 4);
  if (!table?.ranges || table.ranges.length === 0) return lut;
  const noDataBelow = finite(table.no_data_below) ? table.no_data_below : DEFAULT_NO_DATA_BELOW;
  const fadeFloor = noDataBelow >= Math.min(...table.ranges.map((r) => r.min));
  const stride = types ? types.stride : 256;
  const classes = types ? types.classes.length : 1;
  for (let cls = 0; cls < classes; cls++) {
    // no_data_below applies to the rain ramp only, as on the server and in buildLut.
    const segments = rampSegments(classRanges(table, types, cls), cls === 0 ? noDataBelow : -Infinity);
    for (let q = 1; q < stride; q++) {
      const code = cls * stride + q;
      if (code > 255) break;
      const value = Math.round((offset + q * scale) * 1e6) / 1e6;
      const rgba = smoothRampColor(segments, value, fadeFloor);
      if (!rgba) continue;
      for (let c = 0; c < 4; c++) lut[code * 4 + c] = Math.max(0, Math.min(255, Math.round(rgba[c] ?? 0)));
    }
  }
  return lut;
}

/** The table a client colours with: stepped (buildLut, the server's PNG colours) or smooth. */
export function buildRampLut(
  table: ColorTable | null | undefined,
  scale: number,
  offset: number,
  types: PackedTypes | undefined,
  smooth: boolean,
): Uint8Array {
  return smooth ? buildSmoothLut(table, scale, offset, types) : buildLut(table, scale, offset, types);
}

/**
 * Colour of a fractional class-local q on a LUT, premultiplied, 0..1: entries floor(q) and the
 * next one blended, both clamped to 1..qmax so the blend never leaves the class's code range.
 * The web shader's smooth path does this per pixel; the app's tiles sample whole q, where it is
 * just the entry.
 */
export function lutBlend(lut: Uint8Array, base: number, q: number, qmax: number): [number, number, number, number] {
  const m = Math.min(qmax, Math.max(1, q));
  const lo = Math.floor(m);
  const hi = Math.min(lo + 1, qmax);
  const t = m - lo;
  const at = (i: number) => {
    const a = lut[i * 4 + 3] / 255;
    return [(lut[i * 4] / 255) * a, (lut[i * 4 + 1] / 255) * a, (lut[i * 4 + 2] / 255) * a, a];
  };
  const c0 = at(base + lo);
  const c1 = at(base + hi);
  return [0, 1, 2, 3].map((k) => c0[k] + (c1[k] - c0[k]) * t) as [number, number, number, number];
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
