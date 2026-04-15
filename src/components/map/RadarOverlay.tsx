import { useMemo } from "react";
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

  const frame = frames[currentFrameIndex];

  const tileUrl = useMemo(() => {
    if (!manifest || !frame) return null;
    return isRainViewerManifest(manifest)
      ? buildRadarTileUrl(manifest.host, frame)
      : buildSelfHostedTileUrl(serverUrl, activeLayer, frame.path);
  }, [manifest, frame, serverUrl, activeLayer, dataSource]);

  if (!tileUrl) return null;

  return (
    <MapLibreGL.RasterSource
      id="radar-source"
      key={tileUrl}
      tileUrlTemplates={[tileUrl]}
      tileSize={RADAR.TILE_SIZE}
      minZoomLevel={RADAR.MIN_ZOOM}
      maxZoomLevel={dataSource === "selfhosted" ? 12 : RADAR.MAX_ZOOM}
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
