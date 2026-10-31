/**
 * Pure bookkeeping for the in-map radar engine (InMapPackedRadar): where colored tiles live on disk,
 * which frames MapLibre may load, when a viewport change needs a new generation, and what to delete.
 * No React, Skia or file system here; the engine does the I/O and feeds the results in.
 *
 * Disk layout, under the app cache:
 *   radar-inmap/<layer>/<frameKey>/<gen>/{z}/{x}/{y}.png
 * A frame is one packed slot colored through one palette. Its MapLibre source reads one `gen`; when
 * the viewport needs tiles that gen lacks, the engine writes a new gen and the source is remounted
 * on the new URL template (a tile MapLibre asked for before it existed is not retried).
 */
import type { TileId } from "./packed";

export const IN_MAP_DIR = "radar-inmap";
/** Tiles written beyond the visible cover, so a small pan finds its tiles already on disk. */
export const IN_MAP_RING = 1;
/** Total colored PNG bytes kept on disk. */
export const IN_MAP_MAX_BYTES = 96 * 1024 * 1024;
/** A source that is not ready yet sits at this zoom and behind a hidden layer, so nothing is requested. */
const PENDING_ZOOM = 22;

const segment = (s: string) => s.replace(/[^A-Za-z0-9._-]/g, "_");

export const tileKey = (t: TileId) => `${t.z}/${t.x}/${t.y}`;

/** 32-bit FNV-1a of a LUT, so a palette change is a different frame key (and a different directory). */
export function lutHash(bytes: Uint8Array): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < bytes.length; i++) {
    h ^= bytes[i];
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

/** One packed slot under one palette. `v` makes a rewritten bucket a new key. */
export function frameKeyOf(frame: { bucket: string; v: number; slot: number }, lutKeyHash: string): string {
  return `${segment(frame.bucket)}.v${frame.v}.s${frame.slot}.${lutKeyHash}`;
}

/** `layer/frameKey`, the directory that holds all gens of one frame. */
export const frameDir = (layer: string, frameKey: string) => `${segment(layer)}/${frameKey}`;

export const genDir = (layer: string, frameKey: string, gen: number) => `${frameDir(layer, frameKey)}/${gen}`;

/** Path of one tile relative to the in-map root. */
export const tilePath = (layer: string, frameKey: string, gen: number, t: TileId) =>
  `${genDir(layer, frameKey, gen)}/${t.z}/${t.x}/${t.y}.png`;

/** The in-map root as a `file://` URI with a trailing slash, from the app cache directory URI. */
export function inMapRootUri(cacheUri: string): string {
  return `${cacheUri.replace(/\/+$/, "")}/${IN_MAP_DIR}/`;
}

/** MapLibre tile URL template for one gen of one frame. */
export const tileUrlTemplate = (rootUri: string, layer: string, frameKey: string, gen: number) =>
  `${rootUri}${genDir(layer, frameKey, gen)}/{z}/{x}/{y}.png`;

/**
 * The visible cover plus a ring of tiles around it, same zoom, clamped to the world. The cover's
 * own order (nearest the center first) is kept; ring tiles follow.
 */
export function expandCover(cover: readonly TileId[], ring = IN_MAP_RING): TileId[] {
  if (cover.length === 0) return [];
  const z = cover[0].z;
  const n = Math.pow(2, z);
  let x0 = Infinity;
  let x1 = -Infinity;
  let y0 = Infinity;
  let y1 = -Infinity;
  for (const t of cover) {
    x0 = Math.min(x0, t.x);
    x1 = Math.max(x1, t.x);
    y0 = Math.min(y0, t.y);
    y1 = Math.max(y1, t.y);
  }
  const seen = new Set(cover.map(tileKey));
  const out = [...cover];
  for (let y = Math.max(0, y0 - ring); y <= Math.min(n - 1, y1 + ring); y++) {
    for (let x = Math.max(0, x0 - ring); x <= Math.min(n - 1, x1 + ring); x++) {
      const t = { z, x, y };
      if (!seen.has(tileKey(t))) out.push(t);
    }
  }
  return out;
}

/** What a frame has committed on disk: its gen and each tile's PNG size. */
export interface FrameDisk {
  gen: number;
  tiles: ReadonlyMap<string, number>;
}

export const diskBytes = (disk: FrameDisk | undefined) => {
  let sum = 0;
  disk?.tiles.forEach((bytes) => {
    sum += bytes;
  });
  return sum;
};

export interface GenPlan {
  /** The gen to write; the committed gen + 1. */
  gen: number;
  /** Every tile the new gen holds: the cover and its ring. */
  want: TileId[];
  /** Wanted tiles already on disk in the committed gen; copied, not recolored. */
  reuse: TileId[];
  /** Wanted tiles to color and write. */
  render: TileId[];
}

/**
 * The work to make a frame's disk state hold the cover and its ring, or null when it already does.
 * Panning inside the ring needs no new gen; reaching its edge does, so the ring refills before the
 * visible area runs out of tiles.
 */
export function planGen(disk: FrameDisk | undefined, cover: readonly TileId[], ring = IN_MAP_RING): GenPlan | null {
  const want = expandCover(cover, ring);
  if (want.length === 0) return null;
  const have = disk?.tiles;
  const render = have ? want.filter((t) => !have.has(tileKey(t))) : want;
  if (disk && render.length === 0) return null;
  return {
    gen: (disk?.gen ?? 0) + 1,
    want,
    reuse: have ? want.filter((t) => have.has(tileKey(t))) : [],
    render,
  };
}

/** The disk state after a plan's tiles are all on disk; `written` is tileKey → PNG bytes for `plan.render`. */
export function commitGen(disk: FrameDisk | undefined, plan: GenPlan, written: ReadonlyMap<string, number>): FrameDisk {
  const tiles = new Map<string, number>();
  for (const t of plan.want) {
    const key = tileKey(t);
    const bytes = written.get(key) ?? disk?.tiles.get(key);
    if (bytes != null) tiles.set(key, bytes);
  }
  return { gen: plan.gen, tiles };
}

/**
 * "pending": nothing on disk, MapLibre must not load it. "ready": the cover is on disk.
 * "stale": on disk for an earlier viewport, kept mounted while the next gen is written.
 */
export function frameStatus(disk: FrameDisk | undefined, cover: readonly TileId[]): "pending" | "ready" | "stale" {
  if (!disk) return "pending";
  return cover.every((t) => disk.tiles.has(tileKey(t))) ? "ready" : "stale";
}

/** Frame indices in the order to color them: the current one, then outward by distance, newer first. */
export function priorityOrder(indices: readonly number[], current: number): number[] {
  const rest = indices.filter((i) => i !== current);
  rest.sort((a, b) => Math.abs(a - current) - Math.abs(b - current) || b - a);
  return indices.includes(current) ? [current, ...rest] : rest;
}

export interface InMapSlotSpec {
  sourceId: string;
  layerId: string;
  tileUrl: string;
  minZoom: number;
  maxZoom: number;
  /** False keeps the layer hidden so MapLibre requests no tile for the source. */
  visible: boolean;
}

/**
 * The MapLibre source and layer for one frame slot. Always yields a spec (a pending frame gets an
 * inert one) so the map's native child count only changes with the frame count, never as frames
 * become ready. Ids change with the gen: RasterSource ids are frozen on mount, so a new gen is a
 * replace-in-place remount.
 */
export function slotSpec(
  index: number,
  frame: { layer: string; frameKey: string; minZoom: number; maxZoom: number },
  disk: FrameDisk | undefined,
  rootUri: string,
): InMapSlotSpec {
  const base = `inmap-${index}-${frame.frameKey}`;
  if (!disk) {
    return {
      sourceId: `${base}-pending`,
      layerId: `${base}-pending-layer`,
      tileUrl: `${rootUri}pending/{z}/{x}/{y}.png`,
      minZoom: PENDING_ZOOM,
      maxZoom: PENDING_ZOOM,
      visible: false,
    };
  }
  return {
    sourceId: `${base}-g${disk.gen}`,
    layerId: `${base}-g${disk.gen}-layer`,
    tileUrl: tileUrlTemplate(rootUri, frame.layer, frame.frameKey, disk.gen),
    minZoom: frame.minZoom,
    maxZoom: frame.maxZoom,
    visible: true,
  };
}

/** A frame (all its gens) as found on disk. */
export interface DiskFrameListing {
  layer: string;
  frameKey: string;
  gens: readonly number[];
}

/**
 * Paths (relative to the in-map root) to delete: frames no longer wanted, and for the rest every
 * gen but the ones in use. `keep` maps `frameDir(layer, frameKey)` to the gens to keep.
 */
export function selectCleanup(listing: readonly DiskFrameListing[], keep: ReadonlyMap<string, ReadonlySet<number>>): string[] {
  const out: string[] = [];
  for (const f of listing) {
    const kept = keep.get(`${f.layer}/${f.frameKey}`);
    if (!kept) {
      out.push(`${f.layer}/${f.frameKey}`);
      continue;
    }
    for (const gen of f.gens) {
      if (!kept.has(gen)) out.push(`${f.layer}/${f.frameKey}/${gen}`);
    }
  }
  return out;
}

export interface CapEntry {
  key: string;
  bytes: number;
  /** Lower is more worth keeping (the current frame is 0). */
  rank: number;
}

/** Keys to drop, least worth keeping first, until the total fits `capBytes`. Protected keys stay. */
export function selectCapEvictions(entries: readonly CapEntry[], capBytes: number, protectedKeys: ReadonlySet<string> = new Set()): string[] {
  let total = entries.reduce((sum, e) => sum + e.bytes, 0);
  const out: string[] = [];
  const order = entries.filter((e) => !protectedKeys.has(e.key)).sort((a, b) => b.rank - a.rank);
  for (const e of order) {
    if (total <= capBytes) break;
    out.push(e.key);
    total -= e.bytes;
  }
  return out;
}
