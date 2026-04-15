import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useManifest } from "../../hooks/useManifest";
import { buildRadarTileUrl, buildSelfHostedTileUrl } from "../../lib/tileUrl";
import { RADAR } from "../../lib/constants";
import type { RainViewerManifest } from "../../types/weather";

function isRainViewerManifest(m: unknown): m is RainViewerManifest {
  return typeof m === "object" && m !== null && "host" in m;
}

export function RadarOverlay() {
  const { data: manifest } = useManifest();
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);

  if (!manifest || frames.length === 0 || currentFrameIndex < 0) {
    return null;
  }

  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  const tileUrl = isRainViewerManifest(manifest)
    ? buildRadarTileUrl(manifest.host, frame)
    : buildSelfHostedTileUrl(serverUrl, activeLayer, frame.path);

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
