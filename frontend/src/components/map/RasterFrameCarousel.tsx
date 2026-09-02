import { Layer, RasterSource } from "@maplibre/maplibre-react-native";
import { assignSlots, CAROUSEL_WINDOW, clampWindow, type PlaybackWindow } from "../../lib/radarCarousel";
import type { RadarFrame } from "../../types/weather";

export interface CarouselFrameSpec {
  tileUrl: string;
  maxZoom: number;
  /** Distinguishes otherwise-identical frames (palette, layer) in source ids. */
  variant: string;
}

interface Props {
  idPrefix: string;
  frames: RadarFrame[];
  currentFrameIndex: number;
  playbackWindow: PlaybackWindow | null;
  /** Opacity of the visible slot; hidden slots are always 0. */
  opacity: number;
  minZoom: number;
  specFor: (frame: RadarFrame) => CarouselFrameSpec;
}

/**
 * CAROUSEL_WINDOW raster sources, always mounted while frames exist; the current
 * frame is shown by opacity, the rest prefetch at opacity 0 (visible so MapLibre
 * still fetches their tiles). Each tick remounts exactly one HIDDEN slot.
 *
 * Returns a keyed array, never a Fragment with conditional children: the native
 * child count under MLRNMapView must not churn (iOS NSRangeException in
 * insertReactSubview:atIndex:). Empty frames → null, matching the pre-carousel
 * overlay; that transition happens once per manifest/layer load, not per tick.
 */
export function RasterFrameCarousel({
  idPrefix,
  frames,
  currentFrameIndex,
  playbackWindow,
  opacity,
  minZoom,
  specFor,
}: Props) {
  if (frames.length === 0 || currentFrameIndex < 0) return null;
  const clampedIndex = Math.min(currentFrameIndex, frames.length - 1);

  const { start, end } = clampWindow(playbackWindow, frames.length);
  const { slots, visibleSlot } = assignSlots(clampedIndex, start, end, CAROUSEL_WINDOW);

  return slots.map((frameIndex, slot) => {
    // assignSlots only yields indices inside the clamped window; the fallback
    // exists so a slot can never render null (constant child count).
    const frame = frames[frameIndex] ?? frames[clampedIndex];
    const { tileUrl, maxZoom, variant } = specFor(frame);
    // Unique per assignment: RasterSource ids are frozen on mount, and iOS can
    // reuse a stale native source if a replacement briefly shares the id.
    const sourceId = `${idPrefix}-src-${slot}-${variant}-${frame.path}`;
    const isVisible = slot === visibleSlot;

    return (
      <RasterSource
        id={sourceId}
        key={sourceId}
        tiles={[tileUrl]}
        tileSize={256}
        minzoom={minZoom}
        maxzoom={maxZoom}
      >
        <Layer
          type="raster"
          id={`${idPrefix}-layer-${slot}-${variant}-${frame.path}`}
          paint={{
            "raster-opacity": isVisible ? opacity : 0,
            "raster-fade-duration": 0,
          }}
        />
      </RasterSource>
    );
  });
}
