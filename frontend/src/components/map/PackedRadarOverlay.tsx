/**
 * GPU radar: draws packed data tiles (docs/reference/packed-tiles.md) on a Skia canvas over the map, for
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
import { Canvas, Picture, Skia, type SkShader, type SkSize } from "@shopify/react-native-skia";
import { useEffect, useMemo, useReducer } from "react";
import { StyleSheet } from "react-native";
import { useDerivedValue, useSharedValue } from "react-native-reanimated";
import { useIsFocused } from "expo-router";
import { useAppActive } from "../../hooks/useAppActive";
import {
  bucketsInPlaybackOrder,
  PACKED_TILE_SIZE,
  packedTileUrl,
  playbackOrder,
  tileCover,
  type TileId,
} from "../../lib/packed";
import type { PackedFrameLocation, PackedFrames } from "../../hooks/usePackedFrames";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { atlasBytes, CACHE_BUDGET_BYTES, makeLutShaders, PACKED_SKSL, tiles } from "./packedGpu";
import { packedZoomBounds, usePackedCoverView } from "./usePackedCoverView";
import type { SharedCamera } from "./WindParticlesOverlay";

const zoomRangeKey = (loc: PackedFrameLocation) => `${loc.meta.min_zoom}|${loc.meta.max_zoom}`;
const tileKey = (layer: string, bucket: string, t: TileId) => `${layer}/${bucket}/${t.z}/${t.x}/${t.y}`;

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
  const entry = tiles.get(url);
  // An older version can be shorter than this slot's block; clamp sampling would smear its edge.
  if (entry?.state !== "ready" || entry.height < y0 + PACKED_TILE_SIZE) return null;
  tiles.touch(url, entry);
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

  useEffect(() => tiles.subscribe(bumpLoaded), []);

  useEffect(() => {
    opacity.set(radarOpacity);
  }, [radarOpacity, opacity]);

  const effect = useMemo(() => Skia.RuntimeEffect.Make(PACKED_SKSL), []);

  const lutShaders = useMemo(() => makeLutShaders(packed.luts), [packed.luts]);

  const zoomBounds = useMemo(() => packedZoomBounds(packed.locations), [packed.locations]);
  const view = usePackedCoverView(camera, size, zoomBounds);

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
    tiles.setWanted(urls, keys);
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
      if (tiles.get(exactUrl)?.state === "empty") continue;
      const own = readyFor(exactUrl, current.y0) ?? readyFor(tiles.newestReadyUrl(tileKey(current.layer, current.bucket, t)), current.y0);
      if (own) {
        out.push({ ...base, atlas: own.handle.shader, sz: t.z, sx: t.x, sy: t.y });
        continue;
      }
      for (let z = t.z - 1; z >= current.meta.min_zoom; z--) {
        const shift = t.z - z;
        const a = { z, x: t.x >> shift, y: t.y >> shift };
        const parentUrl = urlFor(current, a);
        if (tiles.get(parentUrl)?.state === "empty") break;
        const parent = readyFor(parentUrl, current.y0) ?? readyFor(tiles.newestReadyUrl(tileKey(current.layer, current.bucket, a)), current.y0);
        if (parent) {
          out.push({ ...base, atlas: parent.handle.shader, sz: a.z, sx: a.x, sy: a.y });
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
