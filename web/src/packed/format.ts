/**
 * Packed data tiles v1 (docs/packed-tiles.md): manifest types and the pure math the
 * WebGL layer needs. Nothing here touches the DOM or GL, so all of it is unit-tested.
 */

export interface PackedBucket {
  id: string;
  /** Bucket version; it goes into the tile URL so every update is a new URL. */
  v: number;
  /** Slot timestamps, the same strings as the layer's `frames[].timestamp`. */
  slots: string[];
}

export interface PackedMeta {
  version: number;
  tile_size: number;
  min_zoom: number;
  max_zoom: number;
  cols: number;
  max_slots: number;
  color_key: string;
  categorical: boolean;
  scale: number;
  offset: number;
  url_template: string;
  buckets: PackedBucket[];
}

export interface ColorRange {
  min: number;
  max: number;
  rgba: [number, number, number, number];
}

export interface ColorTable {
  unit?: string;
  ranges: ColorRange[];
  no_data_below?: number;
}

/** True when the client can draw this metadata (v1, continuous values). */
export function usablePacked(meta: PackedMeta | undefined): meta is PackedMeta {
  return Boolean(meta && meta.version === 1 && !meta.categorical && meta.tile_size > 0 && meta.cols > 0 && Array.isArray(meta.buckets));
}

/** 256 x 1 RGBA table, straight (not premultiplied) alpha: entry q colors `offset + q * scale`. */
export function buildLut(table: ColorTable | undefined, scale: number, offset: number): Uint8Array {
  const lut = new Uint8Array(256 * 4);
  if (!table) return lut;
  const noDataBelow = table.no_data_below ?? -999;
  for (let q = 1; q < 256; q++) {
    // Round off float noise (0.05 * 20 = 1.0000000000000002) so band edges compare exactly.
    const value = Math.round((offset + q * scale) * 1e6) / 1e6;
    if (value < noDataBelow) continue;
    let hit: ColorRange | undefined;
    for (const r of table.ranges) if (value >= r.min && value < r.max) hit = r;
    if (hit) lut.set(hit.rgba, q * 4);
  }
  return lut;
}

/** Inverse of the server's quantization; 0 means no data. */
export function quantize(value: number, scale: number, offset: number, floor = -Infinity): number {
  if (Number.isNaN(value) || value < floor) return 0;
  const q = Math.floor((value - offset) / scale);
  return q < 1 ? 0 : Math.min(q, 255);
}

export interface SlotAddress {
  block: number;
  /** 0 = R, 1 = G, 2 = B. */
  channel: number;
  /** Top-left pixel of the slot's block in the atlas. */
  x0: number;
  y0: number;
}

export function slotAddress(slot: number, cols = 2, tile = 512): SlotAddress {
  const block = Math.floor(slot / 3);
  return { block, channel: slot % 3, x0: (block % cols) * tile, y0: Math.floor(block / cols) * tile };
}

/** Atlas pixel size for a bucket with `slots` slots. */
export function atlasSize(slots: number, cols = 2, tile = 512): { width: number; height: number } {
  const blocks = Math.max(1, Math.ceil(slots / 3));
  return { width: tile * cols, height: tile * Math.ceil(blocks / cols) };
}

/**
 * The q values of one slot from decoded atlas pixels (`channels` = 3 for RGB, 4 for RGBA).
 * Pixels outside a short image read as 0.
 */
export function extractSlot(pixels: Uint8Array, width: number, height: number, channels: number, slot: number, cols = 2, tile = 512): Uint8Array {
  const { channel, x0, y0 } = slotAddress(slot, cols, tile);
  const out = new Uint8Array(tile * tile);
  for (let py = 0; py < tile; py++) {
    const y = y0 + py;
    if (y >= height) break;
    for (let px = 0; px < tile; px++) {
      const x = x0 + px;
      if (x >= width) break;
      out[py * tile + px] = pixels[(y * width + x) * channels + channel];
    }
  }
  return out;
}

export interface FrameLocation {
  bucket: string;
  v: number;
  slot: number;
  /** The bucket's slot count in this manifest; a tile fetched for it holds at least these slots. */
  slots: number;
}

export function locateFrame(meta: PackedMeta | undefined, timestamp: string): FrameLocation | null {
  if (!meta) return null;
  for (const b of meta.buckets) {
    const slot = b.slots.indexOf(timestamp);
    if (slot >= 0 && slot < meta.max_slots) return { bucket: b.id, v: b.v, slot, slots: b.slots.length };
  }
  // The spec promises identical strings; fall back to the instant in case the offset is spelled differently.
  const t = Date.parse(timestamp);
  if (Number.isNaN(t)) return null;
  for (const b of meta.buckets) {
    const slot = b.slots.findIndex((s) => Date.parse(s) === t);
    if (slot >= 0 && slot < meta.max_slots) return { bucket: b.id, v: b.v, slot, slots: b.slots.length };
  }
  return null;
}

export interface TileId {
  z: number;
  x: number;
  y: number;
}

export interface Bounds {
  west: number;
  south: number;
  east: number;
  north: number;
}

const MAX_LAT = 85.0511287798066;

export const mercX = (lng: number) => (lng + 180) / 360;
export function mercY(lat: number): number {
  const s = Math.sin((Math.max(-MAX_LAT, Math.min(MAX_LAT, lat)) * Math.PI) / 180);
  return 0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI);
}

export const tileZoom = (zoom: number, minZoom: number, maxZoom: number) => Math.max(minZoom, Math.min(maxZoom, Math.floor(zoom)));

/**
 * 512-px tiles covering `bounds` at clamp(floor(zoom), min, max). Longitudes past the
 * antimeridian clamp to the world edge, so world copies are not drawn. A cover larger than
 * `maxTiles` (a pitched view) steps down a zoom until it fits or reaches `minZoom`.
 */
export function tileCover(bounds: Bounds, zoom: number, minZoom: number, maxZoom: number, maxTiles = Infinity): TileId[] {
  let z = tileZoom(zoom, minZoom, maxZoom);
  let tiles = coverAt(bounds, z);
  while (tiles.length > maxTiles && z > minZoom) tiles = coverAt(bounds, --z);
  return tiles;
}

function coverAt(bounds: Bounds, z: number): TileId[] {
  const n = 2 ** z;
  const clampTile = (v: number) => Math.max(0, Math.min(n - 1, Math.floor(v * n)));
  const west = Math.max(-180, Math.min(bounds.west, bounds.east));
  const east = Math.min(180, Math.max(bounds.west, bounds.east));
  if (east <= -180 || west >= 180) return [];
  const x0 = clampTile(mercX(west));
  const x1 = clampTile(mercX(east));
  const y0 = clampTile(mercY(Math.max(bounds.north, bounds.south)));
  const y1 = clampTile(mercY(Math.min(bounds.north, bounds.south)));
  const tiles: TileId[] = [];
  for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) tiles.push({ z, x, y });
  return tiles;
}

export function packedTileUrl(template: string, layer: string, bucket: string, tile: TileId, v: number): string {
  const vars: Record<string, string> = {
    layer: encodeURIComponent(layer),
    bucket: encodeURIComponent(bucket),
    z: String(tile.z),
    x: String(tile.x),
    y: String(tile.y),
    v: String(v),
  };
  return template.replace(/\{(layer|bucket|z|x|y|v)\}/g, (_, k: string) => vars[k]);
}

/**
 * Column-major 4x4 matrix taking tile-local 0..1 to clip space, given MapLibre's
 * `mainMatrix` (mercator 0..1 to clip). Computed in float64 so deep zooms stay sharp.
 */
export function tileMatrix(main: ArrayLike<number>, tile: TileId): Float32Array {
  const s = 1 / 2 ** tile.z;
  const tx = tile.x * s;
  const ty = tile.y * s;
  const m = new Float32Array(16);
  for (let r = 0; r < 4; r++) {
    m[r] = main[r] * s;
    m[4 + r] = main[4 + r] * s;
    m[8 + r] = main[8 + r];
    m[12 + r] = main[r] * tx + main[4 + r] * ty + main[12 + r];
  }
  return m;
}
