import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import type { LayerType, RadarFrame } from "../../types/weather";
import { LAYERS } from "../../lib/constants";
import type { CarouselFrameSpec } from "./RasterFrameCarousel";
import { RasterOverlay } from "./RasterOverlay";

interface Props {
  layerId: LayerType;
  opacity?: number;
  /** `isPacked[i]`: frame i draws on InMapPackedRadar, so the carousel skips it. */
  isPacked?: readonly boolean[];
}

export function WeatherLayerOverlay({ layerId, opacity = 0.7, isPacked }: Props) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activePalette = useWeatherStore((s) => s.activePalette);

  const layerConfig = LAYERS.find((l) => l.id === layerId);
  if (!layerConfig) return null;

  const specFor = (frame: RadarFrame): CarouselFrameSpec => ({
    tileUrl: buildSelfHostedTileUrl(serverUrl, layerId, frame.path, activePalette),
    // Model pyramids stop at the frame's max_zoom (z6 for HRRR/AQM); deeper requests are 404s.
    maxZoom: frame.maxZoom ?? layerConfig.maxZoom,
    variant: `${layerId}-${activePalette}`,
  });

  return (
    <RasterOverlay
      idPrefix={layerId}
      opacity={opacity}
      minZoom={layerConfig.minZoom}
      specFor={specFor}
      isPacked={isPacked}
    />
  );
}
