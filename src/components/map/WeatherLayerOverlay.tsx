import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import type { LayerType } from "../../types/weather";
import { LAYERS } from "../../lib/constants";

interface Props {
  layerId: LayerType;
  opacity?: number;
}

export function WeatherLayerOverlay({ layerId, opacity = 0.7 }: Props) {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activePalette = useWeatherStore((s) => s.activePalette);

  if (frames.length === 0 || currentFrameIndex < 0) return null;
  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  const layerConfig = LAYERS.find((l) => l.id === layerId);
  if (!layerConfig) return null;

  const tileUrl = buildSelfHostedTileUrl(serverUrl, layerId, frame.path, activePalette);

  return (
    <MapLibreGL.RasterSource
      id={`${layerId}-source`}
      key={`${layerId}-${activePalette}-${frame.path}`}
      tileUrlTemplates={[tileUrl]}
      tileSize={256}
      minZoomLevel={layerConfig.minZoom}
      maxZoomLevel={layerConfig.maxZoom}
    >
      <MapLibreGL.RasterLayer
        id={`${layerId}-layer`}
        style={{ rasterOpacity: opacity, rasterFadeDuration: 0 }}
      />
    </MapLibreGL.RasterSource>
  );
}
