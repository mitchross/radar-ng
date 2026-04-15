import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useManifest } from "../../hooks/useManifest";
import { buildRadarTileUrl } from "../../lib/tileUrl";
import { RADAR } from "../../lib/constants";

export function RadarOverlay() {
  const { data: manifest } = useManifest();
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);

  if (!manifest || frames.length === 0 || currentFrameIndex < 0) {
    return null;
  }

  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  const tileUrl = buildRadarTileUrl(manifest.host, frame);

  return (
    <MapLibreGL.RasterSource
      id="radar-source"
      key={frame.path}
      tileUrlTemplates={[tileUrl]}
      tileSize={RADAR.TILE_SIZE}
      minZoomLevel={RADAR.MIN_ZOOM}
      maxZoomLevel={RADAR.MAX_ZOOM}
    >
      <MapLibreGL.RasterLayer
        id="radar-layer"
        style={{
          rasterOpacity: radarVisible ? radarOpacity : 0,
          rasterFadeDuration: 0,
        }}
      />
    </MapLibreGL.RasterSource>
  );
}
