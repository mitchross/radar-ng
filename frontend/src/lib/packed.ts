/**
 * Packed data tiles (docs/packed-tiles.md): pure helpers shared by the GPU radar overlay.
 *
 * A packed tile is an RGB PNG atlas of quantized frames (3 slots per pixel). The overlay colors a
 * slot through a 256 x 1 lookup table, so playback changes a uniform instead of fetching tiles.
 */
import type { PlaybackWindow } from "./radarCarousel";

export const PACKED_TILE_SIZE = 512;

export interface PackedBucket {
  id: string;
  v: number;
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
  categorical: boolean;
  scale: number;
  offset: number;
  url_template: string;
  buckets: PackedBucket[];
}

export interface PackedColorRange {
  min: number;
  max: number;
  rgba: [number, number, number, number] | number[];
}

/** One color key of `/api/palettes`. */
export interface PackedColorTable {
  ranges?: PackedColorRange[];
  no_data_below?: number;
}

const DEFAULT_NO_DATA_BELOW = -999;

const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
const posInt = (v: unknown): v is number => Number.isInteger(v) && (v as number) > 0;

/**
 * The layer's packed metadata when this client can draw it, else null (raster fallback).
 * Lenient on purpose: bad packed metadata must not fail the whole manifest.
 */
export function parsePackedMeta(value: unknown): PackedMeta | null {
  if (typeof value !== "object" || value === null) return null;
  const m = value as Record<string, unknown>;
  if (m.version !== 1 || m.tile_size !== PACKED_TILE_SIZE || m.categorical === true) return null;
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
  return { ...(m as unknown as PackedMeta), categorical: false, buckets };
}

/**
 * 256 x 1 RGBA table, unpremultiplied: entry q colors `value = offset + q * scale`.
 * q = 0, values below `no_data_below` and values in no range are transparent.
 * Ranges are half-open [min, max); a later overlapping range wins (the renderer's rules).
 */
export function buildLut(table: PackedColorTable | null | undefined, scale: number, offset: number): Uint8Array {
  const lut = new Uint8Array(256 * 4);
  const ranges = table?.ranges;
  if (!ranges || ranges.length === 0) return lut;
  const noDataBelow = finite(table.no_data_below) ? table.no_data_below : DEFAULT_NO_DATA_BELOW;
  for (let q = 1; q < 256; q++) {
    const value = offset + q * scale;
    if (value < noDataBelow) continue;
    let rgba: PackedColorRange["rgba"] | null = null;
    for (const r of ranges) {
      if (value >= r.min && value < r.max) rgba = r.rgba;
    }
    if (!rgba) continue;
    for (let c = 0; c < 4; c++) lut[q * 4 + c] = Math.max(0, Math.min(255, Math.trunc(rgba[c] ?? 0)));
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
}

/** The bucket and slot that hold the frame with this timestamp, or null (raster path). */
export function locateFrame(meta: Pick<PackedMeta, "buckets" | "max_slots"> | null | undefined, timestamp: string): PackedLocation | null {
  if (!meta) return null;
  const epoch = Date.parse(timestamp);
  let byEpoch: PackedLocation | null = null;
  for (const b of meta.buckets) {
    for (let slot = 0; slot < b.slots.length && slot < meta.max_slots; slot++) {
      if (b.slots[slot] === timestamp) return { bucket: b.id, v: b.v, slot };
      // Same instant written with another offset spelling ("Z" vs "+00:00").
      if (!byEpoch && Number.isFinite(epoch) && Date.parse(b.slots[slot]) === epoch) {
        byEpoch = { bucket: b.id, v: b.v, slot };
      }
    }
  }
  return byEpoch;
}

export interface TileId {
  z: number;
  x: number;
  y: number;
}

export interface PackedCamera {
  lon: number;
  lat: number;
  zoom: number;
  /** Degrees clockwise; the overlay rotates around the screen center. */
  bearing?: number;
}

/** Web Mercator world pixel of a lon/lat at map zoom (MapLibre: 512-pt tiles). */
export function worldPixel(lon: number, lat: number, zoom: number): { x: number; y: number } {
  const world = PACKED_TILE_SIZE * Math.pow(2, zoom);
  const latRad = Math.max(-1.4844, Math.min(1.4844, (lat * Math.PI) / 180));
  return {
    x: ((lon + 180) / 360) * world,
    y: (0.5 - Math.log(Math.tan(Math.PI / 4 + latRad / 2)) / (2 * Math.PI)) * world,
  };
}

/** Tile zoom the overlay draws at for a map zoom; overzooms past maxZoom. */
export function packedTileZoom(zoom: number, minZoom: number, maxZoom: number): number {
  return Math.max(minZoom, Math.min(maxZoom, Math.floor(zoom)));
}

/**
 * 512-px tiles covering the screen at clamp(floor(zoom), minZoom, maxZoom), nearest to the center
 * first. With a bearing, the cover is the rotated screen's bounding box. No world wrap.
 */
export function tileCover(
  camera: PackedCamera,
  screen: { width: number; height: number },
  minZoom: number,
  maxZoom: number,
): TileId[] {
  if (!(screen.width > 0 && screen.height > 0)) return [];
  const z = packedTileZoom(camera.zoom, minZoom, maxZoom);
  const center = worldPixel(camera.lon, camera.lat, camera.zoom);
  const rad = ((camera.bearing ?? 0) * Math.PI) / 180;
  const cos = Math.abs(Math.cos(rad));
  const sin = Math.abs(Math.sin(rad));
  const hw = (screen.width / 2) * cos + (screen.height / 2) * sin;
  const hh = (screen.width / 2) * sin + (screen.height / 2) * cos;
  const tileWorld = PACKED_TILE_SIZE * Math.pow(2, camera.zoom - z);
  const n = Math.pow(2, z);
  const clampIdx = (i: number) => Math.max(0, Math.min(n - 1, i));
  const x0 = clampIdx(Math.floor((center.x - hw) / tileWorld));
  const x1 = clampIdx(Math.floor((center.x + hw) / tileWorld));
  const y0 = clampIdx(Math.floor((center.y - hh) / tileWorld));
  const y1 = clampIdx(Math.floor((center.y + hh) / tileWorld));
  const tiles: (TileId & { d: number })[] = [];
  for (let y = y0; y <= y1; y++) {
    for (let x = x0; x <= x1; x++) {
      const dx = (x + 0.5) * tileWorld - center.x;
      const dy = (y + 0.5) * tileWorld - center.y;
      tiles.push({ z, x, y, d: dx * dx + dy * dy });
    }
  }
  tiles.sort((a, b) => a.d - b.d);
  return tiles.map(({ z: tz, x, y }) => ({ z: tz, x, y }));
}

/** Screen rect of a tile before the bearing rotation, the screen center being the camera center. */
export function tileScreenRect(
  tile: TileId,
  camera: PackedCamera,
  screen: { width: number; height: number },
): { x: number; y: number; size: number } {
  const center = worldPixel(camera.lon, camera.lat, camera.zoom);
  const size = PACKED_TILE_SIZE * Math.pow(2, camera.zoom - tile.z);
  return {
    x: screen.width / 2 + tile.x * size - center.x,
    y: screen.height / 2 + tile.y * size - center.y,
    size,
  };
}

/** Absolute tile URL from the manifest's `url_template`; the bucket id is percent-encoded. */
export function packedTileUrl(
  serverUrl: string,
  template: string,
  layer: string,
  bucket: string,
  v: number,
  z: number,
  x: number,
  y: number,
): string {
  const path = template
    .replace("{layer}", layer)
    .replace("{bucket}", encodeURIComponent(bucket))
    .replace("{z}", String(z))
    .replace("{x}", String(x))
    .replace("{y}", String(y))
    .replace("{v}", String(v));
  return /^https?:\/\//i.test(path) ? path : `${serverUrl}${path}`;
}

/** Frame indices playback visits, in order. */
export function playbackOrder(window: PlaybackWindow | null, frameCount: number): number[] {
  if (frameCount === 0) return [];
  const last = frameCount - 1;
  const inRange = (i: number) => i >= 0 && i <= last;
  const seq = window?.sequence?.filter(inRange);
  if (seq && seq.length > 0) return seq;
  const start = window ? Math.max(0, Math.min(window.start, last)) : 0;
  const end = window ? Math.max(start, Math.min(window.end, last)) : last;
  const out: number[] = [];
  for (let i = start; i <= end; i++) out.push(i);
  return out;
}

/**
 * The raster carousel's view of a timeline where some frames are packed: it cycles only the
 * frames without a slot. While a packed frame shows, the carousel holds the next raster frame
 * hidden, so HRRR tiles are fetched before playback reaches them. Null = no raster frames.
 */
export function rasterCarouselView(
  isPacked: readonly boolean[],
  currentIndex: number,
  window: PlaybackWindow | null,
): { currentIndex: number; window: PlaybackWindow; visible: boolean } | null {
  const order = playbackOrder(window, isPacked.length);
  const raster = order.filter((i) => !isPacked[i]);
  if (!isPacked[currentIndex] && currentIndex >= 0 && currentIndex < isPacked.length && !raster.includes(currentIndex)) {
    // A scrub to a raster frame outside the loop still shows it.
    raster.push(currentIndex);
    raster.sort((a, b) => a - b);
  }
  if (raster.length === 0) return null;
  const visible = currentIndex >= 0 && currentIndex < isPacked.length && !isPacked[currentIndex];
  let shown = currentIndex;
  if (!visible) {
    const pos = Math.max(0, order.indexOf(currentIndex));
    const ahead = [...order.slice(pos), ...order.slice(0, pos)].find((i) => !isPacked[i]);
    shown = ahead ?? raster[0];
  }
  return {
    currentIndex: shown,
    window: { start: raster[0], end: raster[raster.length - 1], sequence: raster },
    visible,
  };
}

/** Distinct buckets in playback order starting at the current frame (prefetch priority). */
export function bucketsInPlaybackOrder<T extends { layer: string; bucket: string; v: number }>(
  locations: readonly (T | null)[],
  order: readonly number[],
  currentIndex: number,
): T[] {
  const pos = Math.max(0, order.indexOf(currentIndex));
  const rotated = [currentIndex, ...order.slice(pos), ...order.slice(0, pos)];
  const seen = new Set<string>();
  const out: T[] = [];
  for (const i of rotated) {
    const loc = locations[i];
    if (!loc) continue;
    const key = `${loc.layer}/${loc.bucket}/${loc.v}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(loc);
  }
  return out;
}
