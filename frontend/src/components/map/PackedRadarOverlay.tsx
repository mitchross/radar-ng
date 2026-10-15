/**
 * GPU radar: draws packed data tiles (docs/packed-tiles.md) on a Skia canvas over the map, for
 * radar and for the HRRR and AQM layers.
 *
 * Each visible tile is one rect whose SkSL shader reads the frame's slot from the RGB atlas
 * (nearest) and colors it through a 256 x 1 lookup table. A playback tick changes only uniforms;
 * a palette change swaps only the table. Tiles of every bucket the loop needs are prefetched.
 *
 * Camera: the SharedCamera written from MapLibre region events, projected locally in Web
 * Mercator with 512-pt tiles (MapLibre's zoom), screen center = camera center. Bearing rotates
 * the drawing around the center; the radar screen disables pitch while this overlay is on.
 */
import {
  AlphaType,
  Canvas,
  ColorType,
  FilterMode,
  MipmapMode,
  Picture,
  Skia,
  TileMode,
  type SkImage,
  type SkShader,
  type SkSize,
} from "@shopify/react-native-skia";
import { useEffect, useMemo, useReducer, useState } from "react";
import { StyleSheet } from "react-native";
import { useAnimatedReaction, useDerivedValue, useSharedValue } from "react-native-reanimated";
import { scheduleOnRN } from "react-native-worklets";
import { useIsFocused } from "expo-router";
import { useAppActive } from "../../hooks/useAppActive";
import { fetchWithTimeout } from "../../lib/api";
import {
  bucketsInPlaybackOrder,
  PACKED_TILE_SIZE,
  packedTileUrl,
  playbackOrder,
  tileCover,
  type PackedCamera,
  type TileId,
} from "../../lib/packed";
import type { PackedFrameLocation, PackedFrames } from "../../hooks/usePackedFrames";
import { useWeatherStore } from "../../stores/useWeatherStore";
import type { SharedCamera } from "./WindParticlesOverlay";

const SKSL = `
uniform shader atlas;
uniform shader lut;
uniform float2 tileOrigin;
uniform float tileScale;
uniform float2 blockOrigin;
uniform float channel;
uniform float tileSize;
uniform float opacity;

half4 main(float2 pos) {
  float2 texel = clamp(floor((pos - tileOrigin) / tileScale), float2(0.0), float2(tileSize - 1.0));
  float4 t = atlas.eval(blockOrigin + texel + 0.5);
  float q = channel < 0.5 ? t.r : (channel < 1.5 ? t.g : t.b);
  float4 c = lut.eval(float2(floor(q * 255.0 + 0.5) + 0.5, 0.5));
  return half4(c * opacity);
}`;

/** Decoded atlas bytes kept in memory; prefetch stops short of it. */
const CACHE_BUDGET_BYTES = 192 * 1024 * 1024;
const MAX_IN_FLIGHT = 6;
const ERROR_RETRY_MS = 30_000;
/** Also bounds the small 204/error entries that every bucket version adds. */
const MAX_ENTRIES = 2000;

type Entry =
  | { state: "loading" }
  | { state: "empty" }
  | { state: "error"; at: number }
  | { state: "ready"; image: SkImage; shader: SkShader; height: number; bytes: number };

/** Module-level so tiles survive a remount (tab switch, layer toggle). Map order = LRU order. */
const cache = new Map<string, Entry>();
/** layer/bucket/z/x/y → URL of the newest ready version, for showing an older `v` while a new one loads. */
const newestReady = new Map<string, string>();
let cachedBytes = 0;
let wanted: string[] = [];
let wantedSet = new Set<string>();
/** Wanted URL → its newestReady key. */
let wantedKeys = new Map<string, string>();
let inFlight = 0;
const listeners = new Set<() => void>();

const zoomRangeKey = (loc: PackedFrameLocation) => `${loc.meta.min_zoom}|${loc.meta.max_zoom}`;
const tileKey = (layer: string, bucket: string, t: TileId) => `${layer}/${bucket}/${t.z}/${t.x}/${t.y}`;

function touch(url: string, entry: Entry) {
  cache.delete(url);
  cache.set(url, entry);
}

function evict() {
  for (const [url, entry] of cache) {
    if (cachedBytes <= CACHE_BUDGET_BYTES && cache.size <= MAX_ENTRIES) return;
    if (entry.state === "loading" || wantedSet.has(url)) continue;
    if (entry.state === "ready") {
      if (cachedBytes <= CACHE_BUDGET_BYTES) continue;
      cachedBytes -= entry.bytes;
    }
    cache.delete(url);
  }
}

function decode(buf: ArrayBuffer): { image: SkImage; shader: SkShader } | null {
  const lazy = Skia.Image.MakeImageFromEncoded(Skia.Data.fromBytes(new Uint8Array(buf)));
  if (!lazy) return null;
  // Decode now on the JS thread, not on the first draw mid-animation.
  let image = lazy;
  try {
    image = lazy.makeNonTextureImage() ?? lazy;
  } catch {}
  const shader = image.makeShaderOptions(TileMode.Clamp, TileMode.Clamp, FilterMode.Nearest, MipmapMode.None);
  return { image, shader };
}

async function load(url: string, key: string) {
  inFlight++;
  try {
    const res = await fetchWithTimeout(url);
    if (res.status === 204) {
      cache.set(url, { state: "empty" });
    } else if (!res.ok) {
      cache.set(url, { state: "error", at: Date.now() });
    } else {
      const decoded = decode(await res.arrayBuffer());
      if (!decoded) {
        cache.set(url, { state: "error", at: Date.now() });
      } else {
        const bytes = decoded.image.width() * decoded.image.height() * 4;
        cache.set(url, { state: "ready", ...decoded, height: decoded.image.height(), bytes });
        newestReady.set(key, url);
        cachedBytes += bytes;
      }
    }
  } catch {
    cache.set(url, { state: "error", at: Date.now() });
  } finally {
    inFlight--;
    evict();
    listeners.forEach((l) => l());
    pump();
  }
}

/** Start wanted loads in priority order, at most MAX_IN_FLIGHT at a time. */
function pump() {
  for (const url of wanted) {
    if (inFlight >= MAX_IN_FLIGHT) return;
    const entry = cache.get(url);
    if (entry && !(entry.state === "error" && Date.now() - entry.at > ERROR_RETRY_MS)) continue;
    cache.set(url, { state: "loading" });
    void load(url, wantedKeys.get(url) ?? url);
  }
}

/** Rough decoded size of a bucket's atlas, from its slot count. */
function atlasBytes(slots: number, cols: number): number {
  const rows = Math.max(1, Math.ceil(Math.ceil(Math.max(1, slots) / 3) / cols));
  return PACKED_TILE_SIZE * cols * PACKED_TILE_SIZE * rows * 4;
}

/** Camera and canvas size when the visible tile range last changed. */
interface CoverView {
  key: string;
  camera: PackedCamera;
  screen: { width: number; height: number };
}

interface DrawItem {
  atlas: SkShader;
  lut: SkShader;
  /** Tile the atlas belongs to (an ancestor while a child loads). */
  sz: number;
  sx: number;
  sy: number;
  /** Tile whose rect is filled. */
  dz: number;
  dx: number;
  dy: number;
  x0: number;
  y0: number;
  channel: number;
}

function readyFor(url: string | undefined, y0: number) {
  if (!url) return null;
  const entry = cache.get(url);
  // An older version can be shorter than this slot's block; clamp sampling would smear its edge.
  if (entry?.state !== "ready" || entry.height < y0 + PACKED_TILE_SIZE) return null;
  touch(url, entry);
  return entry;
}

interface Props {
  packed: PackedFrames;
  camera: SharedCamera;
  /** The active layer's raster opacity; defaults to the radar opacity setting. */
  opacity?: number;
}

export function PackedRadarOverlay(props: Props) {
  const active = useAppActive();
  const focused = useIsFocused();
  if (!active || !focused) return null;
  return <ActivePackedRadar {...props} />;
}

function ActivePackedRadar({ packed, camera, opacity: layerOpacity }: Props) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);
  const radarOpacity = useWeatherStore((s) => layerOpacity ?? s.radarOpacity);

  const size = useSharedValue<SkSize>({ width: 0, height: 0 });
  const opacity = useSharedValue(radarOpacity);
  const drawItems = useSharedValue<DrawItem[]>([]);
  const [loadedTick, bumpLoaded] = useReducer((n: number) => n + 1, 0);
  const [view, setView] = useState<CoverView | null>(null);

  useEffect(() => {
    listeners.add(bumpLoaded);
    return () => {
      listeners.delete(bumpLoaded);
    };
  }, []);

  useEffect(() => {
    opacity.set(radarOpacity);
  }, [radarOpacity, opacity]);

  const effect = useMemo(() => Skia.RuntimeEffect.Make(SKSL), []);

  // One LUT image per (color key, quantization); the bytes are unpremultiplied RGBA.
  const lutShaders = useMemo(() => {
    const out: Record<string, SkShader> = {};
    for (const [key, bytes] of Object.entries(packed.luts)) {
      const image = Skia.Image.MakeImage(
        { width: 256, height: 1, alphaType: AlphaType.Unpremul, colorType: ColorType.RGBA_8888 },
        Skia.Data.fromBytes(bytes),
        256 * 4,
      );
      if (image) {
        out[key] = image.makeShaderOptions(TileMode.Clamp, TileMode.Clamp, FilterMode.Nearest, MipmapMode.None);
      }
    }
    return out;
  }, [packed.luts]);

  // Zoom bounds over every packed layer; a cover change at any of them re-runs the JS cover.
  const zoomBounds = useMemo(() => {
    let lo = Infinity;
    let hi = -Infinity;
    for (const loc of packed.locations) {
      if (!loc) continue;
      lo = Math.min(lo, loc.meta.min_zoom);
      hi = Math.max(hi, loc.meta.max_zoom);
    }
    return { lo, hi };
  }, [packed.locations]);

  // The worklet body is inlined (helpers across the worklet boundary have dropped calls).
  useAnimatedReaction(
    () => {
      "worklet";
      const s = size.get();
      const zoom = camera.zoom.get();
      const z = Math.max(zoomBounds.lo, Math.min(zoomBounds.hi, Math.floor(zoom)));
      const world = 512 * Math.pow(2, zoom);
      const latRad = Math.max(-1.4844, Math.min(1.4844, (camera.lat.get() * Math.PI) / 180));
      const cx = ((camera.lon.get() + 180) / 360) * world;
      const cy = (0.5 - Math.log(Math.tan(Math.PI / 4 + latRad / 2)) / (2 * Math.PI)) * world;
      const rad = (camera.bearing.get() * Math.PI) / 180;
      const hw = (s.width / 2) * Math.abs(Math.cos(rad)) + (s.height / 2) * Math.abs(Math.sin(rad));
      const hh = (s.width / 2) * Math.abs(Math.sin(rad)) + (s.height / 2) * Math.abs(Math.cos(rad));
      const tw = 512 * Math.pow(2, zoom - z);
      const key = `${Math.floor(zoom)}|${z}|${Math.floor((cx - hw) / tw)}|${Math.floor((cx + hw) / tw)}|${Math.floor(
        (cy - hh) / tw,
      )}|${Math.floor((cy + hh) / tw)}|${s.width}x${s.height}`;
      return {
        key,
        camera: { lon: camera.lon.get(), lat: camera.lat.get(), zoom, bearing: camera.bearing.get() },
        screen: { width: s.width, height: s.height },
      };
    },
    (next, prev) => {
      if (next.key !== prev?.key) scheduleOnRN(setView, next);
    },
    [zoomBounds],
  );

  // Tiles in view per layer zoom range, recomputed when the cover key changes.
  const covers = useMemo(() => {
    const out = new Map<string, TileId[]>();
    if (!view) return out;
    for (const loc of packed.locations) {
      if (!loc || out.has(zoomRangeKey(loc))) continue;
      out.set(zoomRangeKey(loc), tileCover(view.camera, view.screen, loc.meta.min_zoom, loc.meta.max_zoom));
    }
    return out;
  }, [view, packed.locations]);
  const coverFor = (loc: PackedFrameLocation) => covers.get(zoomRangeKey(loc)) ?? [];

  const urlFor = (loc: PackedFrameLocation, t: TileId) =>
    packedTileUrl(serverUrl, loc.meta.url_template, loc.layer, loc.bucket, loc.v, t.z, t.x, t.y);

  // Prefetch: the current bucket's tiles first, then each bucket in playback order while the
  // estimated decoded size fits the cache budget.
  useEffect(() => {
    const order = playbackOrder(playbackWindow, packed.locations.length);
    const buckets = bucketsInPlaybackOrder(packed.locations, order, currentFrameIndex);
    const urls: string[] = [];
    const keys = new Map<string, string>();
    let budget = 0;
    buckets.forEach((loc, i) => {
      const tiles = coverFor(loc);
      const slots = loc.meta.buckets.find((b) => b.id === loc.bucket)?.slots.length ?? loc.meta.max_slots;
      budget += tiles.length * atlasBytes(slots, loc.meta.cols);
      if (i > 0 && budget > CACHE_BUDGET_BYTES) return;
      for (const t of tiles) {
        const url = urlFor(loc, t);
        urls.push(url);
        keys.set(url, tileKey(loc.layer, loc.bucket, t));
      }
    });
    wanted = urls;
    wantedSet = new Set(urls);
    wantedKeys = keys;
    pump();
    // urlFor and coverFor only read serverUrl and covers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [packed.locations, playbackWindow, currentFrameIndex, covers, serverUrl]);

  // What to draw for the current frame: each tile, else an older version, else an ancestor.
  const current = packed.locations[currentFrameIndex] ?? null;
  const items = useMemo((): DrawItem[] => {
    void loadedTick;
    if (!current || !effect) return [];
    const lut = lutShaders[current.lutKey];
    if (!lut) return [];
    const out: DrawItem[] = [];
    for (const t of coverFor(current)) {
      const base = { lut, dz: t.z, dx: t.x, dy: t.y, x0: current.x0, y0: current.y0, channel: current.channel };
      const exactUrl = urlFor(current, t);
      if (cache.get(exactUrl)?.state === "empty") continue;
      const own = readyFor(exactUrl, current.y0) ?? readyFor(newestReady.get(tileKey(current.layer, current.bucket, t)), current.y0);
      if (own) {
        out.push({ ...base, atlas: own.shader, sz: t.z, sx: t.x, sy: t.y });
        continue;
      }
      for (let z = t.z - 1; z >= current.meta.min_zoom; z--) {
        const shift = t.z - z;
        const a = { z, x: t.x >> shift, y: t.y >> shift };
        const parentUrl = urlFor(current, a);
        if (cache.get(parentUrl)?.state === "empty") break;
        const parent = readyFor(parentUrl, current.y0) ?? readyFor(newestReady.get(tileKey(current.layer, current.bucket, a)), current.y0);
        if (parent) {
          out.push({ ...base, atlas: parent.shader, sz: a.z, sx: a.x, sy: a.y });
          break;
        }
      }
    }
    return out;
    // urlFor and coverFor only read serverUrl and covers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current, covers, loadedTick, lutShaders, effect, serverUrl]);

  useEffect(() => {
    drawItems.set(items);
  }, [items, drawItems]);

  const picture = useDerivedValue(() => {
    "worklet";
    const s = size.get();
    const recorder = Skia.PictureRecorder();
    const canvas = recorder.beginRecording(Skia.XYWHRect(0, 0, Math.max(1, s.width), Math.max(1, s.height)));
    const list = drawItems.get();
    if (effect && list.length > 0 && s.width > 0 && s.height > 0) {
      const zoom = camera.zoom.get();
      const world = 512 * Math.pow(2, zoom);
      const latRad = Math.max(-1.4844, Math.min(1.4844, (camera.lat.get() * Math.PI) / 180));
      const cx = ((camera.lon.get() + 180) / 360) * world;
      const cy = (0.5 - Math.log(Math.tan(Math.PI / 4 + latRad / 2)) / (2 * Math.PI)) * world;
      const ox = s.width / 2 - cx;
      const oy = s.height / 2 - cy;
      const alpha = opacity.get();
      const paint = Skia.Paint();
      paint.setAntiAlias(false);
      canvas.save();
      canvas.rotate(-camera.bearing.get(), s.width / 2, s.height / 2);
      for (let i = 0; i < list.length; i++) {
        const it = list[i];
        const srcSize = 512 * Math.pow(2, zoom - it.sz);
        const dstSize = 512 * Math.pow(2, zoom - it.dz);
        const shader = effect.makeShaderWithChildren(
          [ox + it.sx * srcSize, oy + it.sy * srcSize, srcSize / 512, it.x0, it.y0, it.channel, 512, alpha],
          [it.atlas, it.lut],
        );
        paint.setShader(shader);
        canvas.drawRect(Skia.XYWHRect(ox + it.dx * dstSize, oy + it.dy * dstSize, dstSize, dstSize), paint);
      }
      canvas.restore();
    }
    return recorder.finishRecordingAsPicture();
  });

  return (
    <Canvas style={[StyleSheet.absoluteFill, styles.canvas]} pointerEvents="none" onSize={size} colorSpace="srgb">
      <Picture picture={picture} />
    </Canvas>
  );
}

const styles = StyleSheet.create({
  canvas: { zIndex: 4 },
});
