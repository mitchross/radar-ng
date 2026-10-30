import { useWeatherStore } from "../../stores/useWeatherStore";
import { RasterFrameCarousel, WEATHER_ANCHOR_ID, type CarouselFrameSpec } from "./RasterFrameCarousel";
import { rasterCarouselView } from "../../lib/packed";
import type { RadarFrame } from "../../types/weather";

interface Props {
  idPrefix: string;
  opacity: number;
  minZoom: number;
  /** The frame's tile URL, zoom ceiling and cache variant; the only thing the overlays differ in. */
  specFor: (frame: RadarFrame) => CarouselFrameSpec;
  /** `isPacked[i]`: frame i draws on PackedRadarOverlay, so the carousel skips it. */
  isPacked?: readonly boolean[];
}

// Slot count + swap mechanics live in RasterFrameCarousel / lib/radarCarousel (CAROUSEL_WINDOW).
// RadarOverlay and WeatherLayerOverlay are this with their own spec.
export function RasterOverlay({ idPrefix, opacity, minZoom, specFor, isPacked }: Props) {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);

  // With packed frames the carousel cycles only the raster ones and stays hidden (but fetching)
  // while a packed frame shows. It unmounts only when no raster frame is left in the loop.
  const view = currentFrameIndex >= 0 && isPacked?.some(Boolean)
    ? rasterCarouselView(isPacked, currentFrameIndex, playbackWindow)
    : undefined;
  if (view === null) return null;

  return (
    <RasterFrameCarousel
      idPrefix={idPrefix}
      frames={frames}
      currentFrameIndex={view?.currentIndex ?? currentFrameIndex}
      playbackWindow={view?.window ?? playbackWindow}
      opacity={view && !view.visible ? 0 : opacity}
      minZoom={minZoom}
      specFor={specFor}
      beforeId={WEATHER_ANCHOR_ID}
    />
  );
}
