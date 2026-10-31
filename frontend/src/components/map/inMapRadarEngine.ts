/**
 * The in-map radar engine behind InMapPackedRadar: colors packed tiles on the GPU into PNG files in
 * the app cache, one generation at a time, so MapLibre can draw them as ordinary raster tiles.
 *
 * Each (frame, tile) is one draw of the same SkSL shader PackedRadarOverlay uses, into a 512 x 512
 * offscreen surface, snapshotted and PNG-encoded. Work runs on one async loop, current frame first,
 * with a short sleep after every tile so panning and playback stay smooth. A frame is "committed"
 * (and only then handed to MapLibre) once every tile of its cover is on disk. Bookkeeping that needs
 * no Skia or file system lives in lib/inMapRadarTiles.ts.
 *
 * State is module-level so frames survive a remount (layer switch, tab switch); the first engine of
 * a process wipes the directory, since the index of what is on disk is not persisted.
 */
import { ImageFormat, Skia, type SkPaint, type SkRuntimeEffect, type SkShader, type SkSurface } from "@shopify/react-native-skia";
import {
  hasPngSignature,
  inMapStats,
  patchInMapStats,
  recordInMapError,
  recordInMapTile,
  resetInMapStats,
  type InMapStage,
  type InMapSurfaceKind,
} from "../../lib/inMapDiag";
import { copyTile, deleteRelative, listFrames, verifyTile, wipeAll, writeTile } from "../../lib/inMapRadarDisk";
import {
  commitGen,
  diskBytes,
  expandCover,
  IN_MAP_MAX_BYTES,
  planGen,
  selectCapEvictions,
  selectCleanup,
  tileKey,
  type FrameDisk,
  type GenPlan,
} from "../../lib/inMapRadarTiles";
import { PACKED_TILE_SIZE, type PackedFrameLocation, type TileId } from "../../lib/packed";
import { atlasBytes, CACHE_BUDGET_BYTES, tiles } from "./packedGpu";

/** Sleep after each colored tile, so the JS thread is free for touches and playback ticks. */
const YIELD_MS = 4;
/** A frame whose atlas tile failed to load is retried after this long. */
const RETRY_MS = 30_000;
/** Wait before deleting superseded gens, so MapLibre has moved to the new URL template. */
const SWEEP_DELAY_MS = 2_500;

export interface EngineFrame {
  loc: PackedFrameLocation;
  /** `frameDir(layer, frameKey)`, the key into the disk index. */
  dir: string;
  frameKey: string;
  lut: SkShader;
  urlFor: (t: TileId) => string;
}

export interface EngineInput {
  effect: SkRuntimeEffect;
  /** Frames to keep colored, current first (priorityOrder). Rank = position here. */
  frames: readonly EngineFrame[];
  /** `frameDir` of every packed frame in the manifest; other frames on disk are deleted. */
  allDirs: ReadonlySet<string>;
  /** Visible tile cover per `min_zoom|max_zoom`; a new Map means the viewport changed. */
  covers: ReadonlyMap<string, readonly TileId[]>;
}

export { inMapStats };

/** frameDir → committed disk state; module-level, see the header. */
const disk = new Map<string, FrameDisk>();
let initialized = false;
let surface: SkSurface | null | undefined;
let surfaceKind: InMapSurfaceKind = "unknown";
let emptyPng: Uint8Array | null = null;

const zoomRangeKey = (loc: PackedFrameLocation) => `${loc.meta.min_zoom}|${loc.meta.max_zoom}`;

function getSurface(): SkSurface | null {
  if (surface === undefined) {
    try {
      const gpu = Skia.Surface.MakeOffscreen(PACKED_TILE_SIZE, PACKED_TILE_SIZE, { colorSpace: "srgb" });
      surface = gpu ?? Skia.Surface.Make(PACKED_TILE_SIZE, PACKED_TILE_SIZE);
      surfaceKind = gpu ? "gpu" : surface ? "cpu" : "none";
      patchInMapStats({ surface: surfaceKind });
      if (!surface) recordInMapError("surface", "no surface could be made");
    } catch (err) {
      surface = null;
      surfaceKind = "none";
      patchInMapStats({ surface: surfaceKind });
      recordInMapError("surface", err);
    }
  }
  return surface;
}

/** False when no offscreen surface can be made; the caller falls back to the Skia canvas overlay. */
export const canRenderInMap = () => getSurface() != null;

/** PNG bytes of the surface. Throws (and records what it got) when they are empty or not a PNG. */
function encode(target: SkSurface): Uint8Array {
  target.flush();
  const image = target.makeImageSnapshot();
  let bytes: Uint8Array;
  try {
    // A GPU snapshot is a texture; read it back to a raster image first, which encodes reliably.
    const raster = image.makeNonTextureImage();
    try {
      bytes = raster ? raster.encodeToBytes(ImageFormat.PNG) : new Uint8Array();
    } finally {
      raster?.dispose();
    }
    if (!hasPngSignature(bytes)) bytes = image.encodeToBytes(ImageFormat.PNG);
  } finally {
    image.dispose();
  }
  const signature = hasPngSignature(bytes);
  patchInMapStats({ lastPngBytes: bytes.length, lastPngSignature: signature });
  if (bytes.length === 0) throw new Error("empty");
  if (!signature) throw new Error("not a PNG");
  return bytes;
}

/** A fully transparent tile: what a 204 (no data) packed tile becomes. */
function transparentPng(target: SkSurface): Uint8Array {
  if (!emptyPng) {
    target.getCanvas().clear(Skia.Color("transparent"));
    emptyPng = encode(target);
  }
  return emptyPng;
}

/** Draw one tile into the surface (the "color" stage); encode() reads it back. */
function colorTile(target: SkSurface, paint: SkPaint, effect: SkRuntimeEffect, atlas: SkShader, frame: EngineFrame) {
  const canvas = target.getCanvas();
  canvas.clear(Skia.Color("transparent"));
  const { x0, y0, channel } = frame.loc;
  // The tile fills the surface 1:1, so the origin is (0, 0) and the scale 1; opacity is applied by MapLibre.
  paint.setShader(effect.makeShaderWithChildren([0, 0, 1, x0, y0, channel, PACKED_TILE_SIZE, 1], [atlas, frame.lut]));
  canvas.drawRect(Skia.XYWHRect(0, 0, PACKED_TILE_SIZE, PACKED_TILE_SIZE), paint);
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** The disk state MapLibre sources are built from. */
export const getInMapDisk = (): ReadonlyMap<string, FrameDisk> => disk;

/** Forget everything and delete it from disk (engine switched away, Fast radar turned off). */
export function clearInMapCache() {
  disk.clear();
  resetInMapStats();
  // The surface outlives the cache, so keep reporting it.
  patchInMapStats({ surface: surfaceKind });
  wipeAll();
}

/** `framesCommitted` is the number of frames in the disk index; call after every change to it. */
function syncFrameCount() {
  if (inMapStats.framesCommitted !== disk.size) patchInMapStats({ framesCommitted: disk.size });
}

export function createInMapEngine(onChange: () => void) {
  if (!initialized) {
    initialized = true;
    wipeAll();
  }
  let input: EngineInput | null = null;
  let stopped = false;
  let running = false;
  let again = false;
  /** Frames not to try again yet: a failed atlas load until the time, or no room under the cap. */
  const holdUntil = new Map<string, number>();
  const noRoom = new Set<string>();
  let activeGen: { dir: string; gen: number } | null = null;
  let sweepTimer: ReturnType<typeof setTimeout> | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let waiters: (() => void)[] = [];

  /** The cache entry for `url` once it has finished loading (null when the engine stops meanwhile). */
  function settled(url: string) {
    const done = () => {
      const entry = tiles.get(url);
      return entry && entry.state !== "loading" ? entry : null;
    };
    const now = done();
    if (now) return Promise.resolve(now);
    return new Promise<ReturnType<typeof done>>((resolve) => {
      const finish = () => {
        off();
        waiters = waiters.filter((w) => w !== finish);
        resolve(done());
      };
      const off = tiles.subscribe(() => {
        if (done() || stopped) finish();
      });
      waiters.push(finish);
    });
  }

  /** Atlas tiles for this frame and the ones after it, within half the decoded-tile budget. */
  function wantAtlases(from: number, current: EngineInput) {
    const urls: string[] = [];
    const keys = new Map<string, string>();
    let budget = 0;
    for (let i = from; i < current.frames.length; i++) {
      const frame = current.frames[i];
      const cover = current.covers.get(zoomRangeKey(frame.loc));
      if (!cover) continue;
      const want = expandCover(cover);
      const bucketUrls = want.map((t) => frame.urlFor(t)).filter((u) => !keys.has(u));
      if (bucketUrls.length === 0) continue;
      const slots = frame.loc.meta.buckets.find((b) => b.id === frame.loc.bucket)?.slots.length ?? frame.loc.meta.max_slots;
      budget += bucketUrls.length * atlasBytes(slots, frame.loc.meta.cols);
      if (i > from && budget > CACHE_BUDGET_BYTES / 2) break;
      want.forEach((t) => {
        const url = frame.urlFor(t);
        if (keys.has(url)) return;
        urls.push(url);
        keys.set(url, `${frame.loc.layer}/${frame.loc.bucket}/${t.z}/${t.x}/${t.y}`);
      });
    }
    tiles.setWanted(urls, keys);
  }

  function totalBytes() {
    let sum = 0;
    disk.forEach((d) => {
      sum += diskBytes(d);
    });
    return sum;
  }

  /** Drop frames ranked behind `rank` until `needed` more bytes fit under the cap. */
  function makeRoom(rank: number, needed: number, current: EngineInput): boolean {
    const total = totalBytes();
    if (total + needed <= IN_MAP_MAX_BYTES) return true;
    const rankOf = new Map(current.frames.map((f, i) => [f.dir, i]));
    const entries = [...disk].map(([key, d]) => ({ key, bytes: diskBytes(d), rank: rankOf.get(key) ?? Infinity }));
    // Only frames behind this one go: dropping an earlier frame would just be rebuilt next.
    const keep = new Set(entries.filter((e) => e.rank <= rank).map((e) => e.key));
    const drop = selectCapEvictions(entries, IN_MAP_MAX_BYTES - needed, keep);
    for (const key of drop) disk.delete(key);
    syncFrameCount();
    deleteRelative(drop);
    if (drop.length > 0) onChange();
    return totalBytes() + needed <= IN_MAP_MAX_BYTES;
  }

  function nextJob(current: EngineInput) {
    const now = Date.now();
    for (let rank = 0; rank < current.frames.length; rank++) {
      const frame = current.frames[rank];
      if (noRoom.has(frame.dir) || (holdUntil.get(frame.dir) ?? 0) > now) continue;
      const cover = current.covers.get(zoomRangeKey(frame.loc));
      if (!cover || cover.length === 0) continue;
      const plan = planGen(disk.get(frame.dir), cover);
      if (plan) return { rank, frame, plan };
    }
    return null;
  }

  /** Check the gen's first tile is on disk where the MapLibre URL template points before the gen is handed over. */
  function verifyGen(layer: string, frame: EngineFrame, plan: GenPlan): boolean {
    const { uri, exists } = verifyTile(layer, frame.frameKey, plan.gen, plan.want[0]);
    if (inMapStats.firstTileExists !== true) patchInMapStats({ firstTileUri: uri, firstTileExists: exists });
    patchInMapStats({ verifyOk: exists });
    if (!exists) {
      recordInMapError("verify", "tile file missing after write");
      onChange(); // the safety net re-checks whether to draw the Classic fast overlay
    }
    return exists;
  }

  /** Write one gen of one frame. "done" = committed; "abort" = the viewport moved or the engine stopped. */
  async function prepare(rank: number, frame: EngineFrame, plan: GenPlan, current: EngineInput): Promise<"done" | "abort" | "error" | "full"> {
    const target = getSurface();
    if (!target) return "error";
    const prev = disk.get(frame.dir);
    const perTile = prev && prev.tiles.size > 0 ? diskBytes(prev) / prev.tiles.size : 24 * 1024;
    if (!makeRoom(rank, plan.render.length * perTile, current)) return "full";
    wantAtlases(rank, current);
    const paint = Skia.Paint();
    paint.setAntiAlias(false);
    const aborted = () => stopped || input?.covers !== current.covers;
    const { layer } = frame.loc;
    const written = new Map<string, number>();
    const toRender = [...plan.render];
    activeGen = { dir: frame.dir, gen: plan.gen };
    let stage: InMapStage = "copy";
    try {
      if (prev) {
        for (const t of plan.reuse) {
          if (aborted()) return "abort";
          if (!(await copyTile(layer, frame.frameKey, prev.gen, plan.gen, t))) toRender.push(t);
        }
      }
      for (const t of toRender) {
        stage = "atlas-load";
        const entry = await settled(frame.urlFor(t));
        if (aborted()) return "abort";
        if (!entry || entry.state === "error") {
          recordInMapError("atlas-load", entry ? "atlas download failed" : "engine stopped");
          return "error";
        }
        // An atlas shorter than this slot's block has no data for it (or is an older version).
        let bytes: Uint8Array;
        if (entry.state === "ready" && entry.height >= frame.loc.y0 + PACKED_TILE_SIZE) {
          tiles.touch(frame.urlFor(t), entry);
          stage = "color";
          colorTile(target, paint, current.effect, entry.handle.shader, frame);
          stage = "encode";
          bytes = encode(target);
        } else {
          stage = "encode";
          bytes = transparentPng(target);
        }
        stage = "write";
        await writeTile(layer, frame.frameKey, plan.gen, t, bytes);
        written.set(tileKey(t), bytes.length);
        recordInMapTile(bytes.length);
        await sleep(YIELD_MS);
      }
      stage = "verify";
      if (!verifyGen(layer, frame, plan)) return "error";
      disk.set(frame.dir, commitGen(prev, plan, written));
      patchInMapStats({ writtenZoom: plan.want[0].z });
      syncFrameCount();
      onChange();
      return "done";
    } catch (err) {
      recordInMapError(stage, err);
      return "error";
    } finally {
      activeGen = null;
    }
  }

  async function loop() {
    running = true;
    try {
      do {
        again = false;
        for (;;) {
          const current = input;
          if (stopped || !current) break;
          const job = nextJob(current);
          if (!job) break;
          const outcome = await prepare(job.rank, job.frame, job.plan, current);
          if (outcome === "error") {
            holdUntil.set(job.frame.dir, Date.now() + RETRY_MS);
            if (retryTimer) clearTimeout(retryTimer);
            retryTimer = setTimeout(kick, RETRY_MS + 100);
          } else if (outcome === "full") {
            noRoom.add(job.frame.dir);
          }
        }
      } while (again && !stopped);
    } finally {
      running = false;
    }
    scheduleSweep();
  }

  function kick() {
    if (stopped) return;
    again = true;
    if (!running) void loop();
  }

  /** Delete frames that left the manifest and every gen a source no longer reads. */
  function scheduleSweep() {
    if (sweepTimer) clearTimeout(sweepTimer);
    sweepTimer = setTimeout(() => {
      sweepTimer = null;
      const current = input;
      if (!current || stopped || running) return;
      for (const key of [...disk.keys()]) if (!current.allDirs.has(key)) disk.delete(key);
      syncFrameCount();
      const keep = new Map<string, ReadonlySet<number>>();
      disk.forEach((d, key) => keep.set(key, new Set([d.gen])));
      if (activeGen) keep.set(activeGen.dir, new Set([...(keep.get(activeGen.dir) ?? []), activeGen.gen]));
      deleteRelative(selectCleanup(listFrames(), keep));
    }, SWEEP_DELAY_MS);
  }

  return {
    /** New frames or a new viewport; starts coloring whatever is missing. */
    update(next: EngineInput) {
      const viewportMoved = input?.covers !== next.covers;
      input = next;
      if (viewportMoved) noRoom.clear();
      kick();
    },
    stop() {
      stopped = true;
      if (sweepTimer) clearTimeout(sweepTimer);
      if (retryTimer) clearTimeout(retryTimer);
      [...waiters].forEach((w) => w());
    },
  };
}
