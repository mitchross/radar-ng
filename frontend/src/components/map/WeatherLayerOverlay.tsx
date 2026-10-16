import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import type { LayerType, RadarFrame } from "../../types/weather";
import { LAYERS } from "../../lib/constants";
import { RasterFrameCarousel, type CarouselFrameSpec } from "./RasterFrameCarousel";
import { rasterCarouselView } from "../../lib/packed";

interface Props {
  layerId: LayerType;
  opacity?: number;
  /** `isPacked[i]`: frame i draws on PackedRadarOverlay, so the carousel skips it. */
  isPacked?: readonly boolean[];
}

// Same carousel as RadarOverlay; only the URL/zoom spec differs.
export function WeatherLayerOverlay({ layerId, opacity = 0.7, isPacked }: Props) {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);

  const layerConfig = LAYERS.find((l) => l.id === layerId);
  // As in RadarOverlay: with packed frames the carousel cycles only the raster ones.
  const view = currentFrameIndex >= 0 && isPacked?.some(Boolean)
    ? rasterCarouselView(isPacked, currentFrameIndex, playbackWindow)
    : undefined;
  if (!layerConfig || view === null) return null;

  const specFor = (frame: RadarFrame): CarouselFrameSpec => ({
    tileUrl: buildSelfHostedTileUrl(serverUrl, layerId, frame.path, activePalette),
    // Model pyramids stop at the frame's max_zoom (z6 for HRRR/AQM); deeper requests are 404s.
    maxZoom: frame.maxZoom ?? layerConfig.maxZoom,
    variant: `${layerId}-${activePalette}`,
  });

  return (
    <RasterFrameCarousel
      idPrefix={layerId}
      frames={frames}
      currentFrameIndex={view?.currentIndex ?? currentFrameIndex}
      playbackWindow={view?.window ?? playbackWindow}
      opacity={view && !view.visible ? 0 : opacity}
      minZoom={layerConfig.minZoom}
      specFor={specFor}
    />
  );
}
