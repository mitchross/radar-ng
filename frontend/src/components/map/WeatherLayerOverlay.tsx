import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import type { LayerType, RadarFrame } from "../../types/weather";
import { LAYERS } from "../../lib/constants";
import { RasterFrameCarousel, type CarouselFrameSpec } from "./RasterFrameCarousel";

interface Props {
  layerId: LayerType;
  opacity?: number;
}

// Same carousel as RadarOverlay; only the URL/zoom spec differs.
export function WeatherLayerOverlay({ layerId, opacity = 0.7 }: Props) {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);

  const layerConfig = LAYERS.find((l) => l.id === layerId);
  if (!layerConfig) return null;

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
      currentFrameIndex={currentFrameIndex}
      playbackWindow={playbackWindow}
      opacity={opacity}
      minZoom={layerConfig.minZoom}
      specFor={specFor}
    />
  );
}
