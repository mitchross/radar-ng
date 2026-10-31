/**
 * In-map radar: the packed tiles of PackedRadarOverlay, drawn by MapLibre itself.
 *
 * Packed tiles are downloaded and colored on the GPU as before (inMapRadarEngine), but each colored
 * tile is saved as a PNG in the app cache and every frame is an ordinary raster source reading
 * `file://.../{z}/{x}/{y}.png`, inserted under WeatherAnchor like the classic raster frames. So
 * warnings, storm cells and the location pin draw above the radar, which the Skia canvas over the
 * map cannot do. Playback only changes `raster-opacity`: every ready frame stays mounted.
 *
 * This relies on MapLibre Native resolving `file://` tile URL templates on iOS and Android (its
 * local file source). It is not verified on a device yet; see the PR.
 *
 * Native child count: one RasterSource + Layer per packed frame, rendered for every frame from the
 * moment the manifest loads. A frame whose tiles are not on disk yet gets an inert pair (hidden
 * layer, zoom 22), so MapLibre never requests a file that does not exist, and becoming ready swaps
 * the pair in place instead of adding children.
 */
import { Layer, RasterSource } from "@maplibre/maplibre-react-native";
import { Skia } from "@shopify/react-native-skia";
import { useEffect, useMemo, useReducer, useRef, useState } from "react";
import { useWindowDimensions } from "react-native";
import { useSharedValue } from "react-native-reanimated";
import { useIsFocused } from "expo-router";
import { useAppActive } from "../../hooks/useAppActive";
import type { PackedFrames } from "../../hooks/usePackedFrames";
import { rootUri } from "../../lib/inMapRadarDisk";
import { frameDir, frameKeyOf, lutHash, priorityOrder, slotSpec, type InMapSlotSpec } from "../../lib/inMapRadarTiles";
import { PACKED_TILE_SIZE, packedTileUrl, playbackOrder, tileCover, type TileId } from "../../lib/packed";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { createInMapEngine, getInMapDisk, type EngineFrame } from "./inMapRadarEngine";
import { makeLutShaders, PACKED_SKSL } from "./packedGpu";
import { WEATHER_ANCHOR_ID } from "./RasterFrameCarousel";
import { packedZoomBounds, usePackedCoverView, type CoverView } from "./usePackedCoverView";
import type { SharedCamera } from "./WindParticlesOverlay";

/** A moved viewport must hold still this long before tiles are colored for it. */
const SETTLE_MS = 250;

interface Props {
  packed: PackedFrames;
  camera: SharedCamera;
  /** The active layer's raster opacity; defaults to the radar opacity setting. */
  opacity?: number;
}

interface FrameSlot {
  index: number;
  spec: InMapSlotSpec;
}

export function InMapPackedRadar({ packed, camera, opacity: layerOpacity }: Props) {
  const active = useAppActive();
  const focused = useIsFocused();
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);
  const radarOpacity = useWeatherStore((s) => layerOpacity ?? s.radarOpacity);
  // Stays mounted in the background (the map keeps its layers); only the coloring pauses.
  const paused = !active || !focused;

  // The radar map is full screen.
  const { width, height } = useWindowDimensions();
  const size = useSharedValue({ width, height });
  useEffect(() => {
    size.set({ width, height });
  }, [width, height, size]);

  const [, bump] = useReducer((n: number) => n + 1, 0);
  const zoomBounds = useMemo(() => packedZoomBounds(packed.locations), [packed.locations]);
  const view = usePackedCoverView(camera, size, zoomBounds);
  const [settled, setSettled] = useState<CoverView | null>(null);
  useEffect(() => {
    if (!view) return;
    // The first view goes straight through; later ones wait for the pan or pinch to stop.
    const id = setTimeout(() => setSettled(view), settled ? SETTLE_MS : 0);
    return () => clearTimeout(id);
  }, [view, settled]);

  const effect = useMemo(() => Skia.RuntimeEffect.Make(PACKED_SKSL), []);
  const lutShaders = useMemo(() => makeLutShaders(packed.luts), [packed.luts]);
  const lutHashes = useMemo(() => {
    const out: Record<string, string> = {};
    for (const [key, bytes] of Object.entries(packed.luts)) out[key] = lutHash(bytes);
    return out;
  }, [packed.luts]);

  // Tiles in view per layer zoom range; a new Map = a new viewport.
  const covers = useMemo(() => {
    const out = new Map<string, TileId[]>();
    if (!settled) return out;
    for (const loc of packed.locations) {
      const key = loc ? `${loc.meta.min_zoom}|${loc.meta.max_zoom}` : "";
      if (!loc || out.has(key)) continue;
      out.set(key, tileCover(settled.camera, settled.screen, loc.meta.min_zoom, loc.meta.max_zoom));
    }
    return out;
  }, [settled, packed.locations]);

  // Every packed frame, whether or not it is being colored.
  const frames = useMemo(() => {
    const out = new Map<number, EngineFrame>();
    packed.locations.forEach((loc, index) => {
      const lut = loc ? lutShaders[loc.lutKey] : undefined;
      if (!loc || !lut) return;
      const frameKey = frameKeyOf(loc, lutHashes[loc.lutKey] ?? "0");
      out.set(index, {
        loc,
        dir: frameDir(loc.layer, frameKey),
        frameKey,
        lut,
        urlFor: (t) => packedTileUrl(serverUrl, loc.meta.url_template, loc.layer, loc.bucket, loc.v, t.z, t.x, t.y),
      });
    });
    return out;
  }, [packed.locations, lutShaders, lutHashes, serverUrl]);

  // Color the loop's frames, the current one first (the newest when the radar opens).
  const engineRef = useRef<ReturnType<typeof createInMapEngine> | null>(null);
  useEffect(() => {
    if (paused) return;
    const engine = createInMapEngine(bump);
    engineRef.current = engine;
    return () => {
      engine.stop();
      engineRef.current = null;
    };
  }, [paused]);
  useEffect(() => {
    if (!effect || covers.size === 0) return;
    const order = playbackOrder(playbackWindow, packed.locations.length);
    const wanted = priorityOrder(
      [...new Set([...order, currentFrameIndex])].filter((i) => frames.has(i)),
      currentFrameIndex,
    );
    engineRef.current?.update({
      effect,
      frames: wanted.map((i) => frames.get(i)!),
      allDirs: new Set([...frames.values()].map((f) => f.dir)),
      covers,
    });
  }, [paused, effect, covers, frames, playbackWindow, currentFrameIndex, packed.locations.length]);

  const root = rootUri();
  const disk = getInMapDisk();
  const slots: FrameSlot[] = [];
  frames.forEach((frame, index) => {
    const { layer, meta } = frame.loc;
    slots.push({
      index,
      spec: slotSpec(index, { layer, frameKey: frame.frameKey, minZoom: meta.min_zoom, maxZoom: meta.max_zoom }, disk.get(frame.dir), root),
    });
  });
  return <InMapFrameLayers slots={slots} currentIndex={currentFrameIndex} opacity={radarOpacity} />;
}

/**
 * One keyed source + raster layer per frame. The current frame shows at `opacity`, the rest at 0
 * (still visible layers, so MapLibre keeps their tiles loaded); no fade, so a tick is instant.
 */
export function InMapFrameLayers({ slots, currentIndex, opacity }: { slots: FrameSlot[]; currentIndex: number; opacity: number }) {
  if (slots.length === 0) return null;
  return slots.map(({ index, spec }) => (
    <RasterSource
      id={spec.sourceId}
      key={spec.sourceId}
      tiles={[spec.tileUrl]}
      tileSize={PACKED_TILE_SIZE}
      minzoom={spec.minZoom}
      maxzoom={spec.maxZoom}
    >
      <Layer
        type="raster"
        id={spec.layerId}
        beforeId={WEATHER_ANCHOR_ID}
        layout={{ visibility: spec.visible ? "visible" : "none" }}
        paint={{
          "raster-opacity": spec.visible && index === currentIndex ? opacity : 0,
          "raster-fade-duration": 0,
          "raster-opacity-transition": { duration: 0, delay: 0 },
        }}
      />
    </RasterSource>
  ));
}
