import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";

export function RadarOverlay() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const activePalette = useWeatherStore((s) => s.activePalette);

  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  // In forecast mode the frame list is a merged radar + nowcast + HRRR stream —
  // per-frame `source` tells us which tile subtree to hit.
  const layerForUrl = frame.source ?? activeLayer;
  const tileUrl = buildSelfHostedTileUrl(serverUrl, layerForUrl, frame.path, activePalette);
  return (
    <MapLibreGL.RasterSource
      id="radar-source"
      key={`${activePalette}-${layerForUrl}-${frame.path}`}
      tileUrlTemplates={[tileUrl]}
      tileSize={256}
      minZoomLevel={1}
      maxZoomLevel={12}
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
