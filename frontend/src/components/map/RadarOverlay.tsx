import { Layer, RasterSource } from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";

// Real pyramid ceiling per product (MRMS 7, nowcast/HRRR 6); a manifest
// `max_zoom` overrides. Overstating it makes MapLibre fetch 404 tiles and paint nothing.
const SOURCE_MAX_ZOOM: Record<string, number> = {
  radar: 7,
  "radar-hrrr": 6,
  nowcast: 6,
};

// Pyramids start at z4 (CONUS only); lower zooms are 404s that starve real tile fetches.
const SOURCE_MIN_ZOOM = 4;

// One RasterSource, remounted per frame (a mounted source cannot change its URL).
// A 7-source preload crashed iOS (NSRangeException in MLRNMapView insertReactSubview:
// child count churned); the 5-slot constant-count carousel from b12012f is the planned fix.
export function RadarOverlay() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const activePalette = useWeatherStore((s) => s.activePalette);

  if (frames.length === 0 || currentFrameIndex < 0) return null;
  const frame = frames[currentFrameIndex];
  if (!frame) return null;

  // In forecast mode the frame list is a merged radar + nowcast + HRRR stream —
  // per-frame `source` tells us which tile subtree to hit.
  const layerForUrl = frame.source ?? activeLayer;
  const tileUrl = buildSelfHostedTileUrl(serverUrl, layerForUrl, frame.path, activePalette);
  const maxZoom = frame.maxZoom ?? SOURCE_MAX_ZOOM[layerForUrl] ?? 7;

  return (
    <RasterSource
      id="radar-source"
      key={`${activePalette}-${layerForUrl}-${frame.path}`}
      tiles={[tileUrl]}
      tileSize={256}
      minzoom={SOURCE_MIN_ZOOM}
      maxzoom={maxZoom}
    >
      <Layer
        type="raster"
        id="radar-layer"
        paint={{
          "raster-opacity": radarVisible ? radarOpacity : 0,
          "raster-fade-duration": 0,
        }}
      />
    </RasterSource>
  );
}
