import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildIEMTileUrl, buildSelfHostedTileUrl } from "../../lib/tileUrl";
import { IEM } from "../../lib/constants";

export function RadarOverlay() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);

  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  if (dataSource !== "selfhosted") {
    // IEM NEXRAD tiles (free, proper NWS colors)
    const tileUrl = buildIEMTileUrl(frame.path);
    return (
      <MapLibreGL.RasterSource
        id="radar-source"
        key={frame.path}
        tileUrlTemplates={[tileUrl]}
        tileSize={256}
        minZoomLevel={IEM.MIN_ZOOM}
        maxZoomLevel={IEM.MAX_ZOOM}
        tms={true}
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
  } else {
    // Self-hosted tiles
    const tileUrl = buildSelfHostedTileUrl(serverUrl, activeLayer, frame.path);
    return (
      <MapLibreGL.RasterSource
        id="radar-source"
        key={frame.path}
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
}
