/**
 * Packed data tiles (docs/reference/packed-tiles.md): pure helpers for the GPU radar overlay. The
 * manifest types, parser, LUT and slot helpers are shared with the web radar in `web/src/shared/packed`;
 * what stays here is the app's camera-based tile cover and the playback glue.
 */
import {
  PACKED_TILE_SIZE,
  buildLut,
  locateFrame,
  parsePackedMeta,
  slotAddress,
  type PackedLocation,
  type PackedMeta,
} from "../../../web/src/shared/packed";
import type { ColorTable } from "../../../web/src/shared/serverPalettes";
import type { PlaybackWindow } from "./radarCarousel";

export {
  PACKED_TILE_SIZE,
  buildLut,
  classRanges,
  locateFrame,
  parsePackedMeta,
  slotAddress,
  type PackedBucket,
  type PackedLocation,
  type PackedMeta,
  type PackedTypes,
  type SlotAddress,
} from "../../../web/src/shared/packed";

export interface PackedFrameLocation extends PackedLocation {
  layer: string;
  meta: PackedMeta;
  /** Key into PackedFrames.luts. */
  lutKey: string;
  channel: number;
  x0: number;
  y0: number;
}

export interface PackedFrames {
  /** Per frame index: where the frame lives in packed tiles, or null (raster path). */
  locations: (PackedFrameLocation | null)[];
  isPacked: boolean[];
  /** 256 x 1 RGBA tables (unpremultiplied) by lutKey; a palette change swaps only these. */
  luts: Record<string, Uint8Array>;
}

const lutKeyOf = (meta: PackedMeta) =>
  `${meta.color_key}|${meta.scale}|${meta.offset}` + (meta.types ? `|${meta.types.stride}|${meta.types.classes.join(",")}` : "");

/**
 * Where each frame lives in packed tiles. A frame's layer is its `source` (the merged radar
 * stream), else `fallbackLayer`. Null when no frame has a slot and a palette table to color it.
 */
export function buildPackedFrames(
  frames: readonly { timestamp: string; source?: string }[],
  layers: Record<string, { packed?: unknown } | undefined>,
  tables: Record<string, unknown> | undefined,
  fallbackLayer: string,
): PackedFrames | null {
  const metas = new Map<string, PackedMeta | null>();
  const luts: Record<string, Uint8Array> = {};
  const metaFor = (layer: string): PackedMeta | null => {
    if (!metas.has(layer)) {
      const meta = parsePackedMeta(layers[layer]?.packed);
      const table = meta ? (tables?.[meta.color_key] as ColorTable | undefined) : undefined;
      // No table for the key: leave the layer on rasters rather than draw it blank.
      const usable = meta && table?.ranges?.length ? meta : null;
      if (usable && table) luts[lutKeyOf(usable)] ??= buildLut(table, usable.scale, usable.offset, usable.types);
      metas.set(layer, usable);
    }
    return metas.get(layer) ?? null;
  };
  const locations = frames.map((frame): PackedFrameLocation | null => {
    const layer = frame.source ?? fallbackLayer;
    const meta = metaFor(layer);
    const loc = locateFrame(meta, frame.timestamp);
    if (!meta || !loc) return null;
    const { channel, x0, y0 } = slotAddress(loc.slot, meta.cols, meta.tile_size);
    return { ...loc, layer, meta, lutKey: lutKeyOf(meta), channel, x0, y0 };
  });
  if (!locations.some(Boolean)) return null;
  return { locations, isPacked: locations.map(Boolean), luts };
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
