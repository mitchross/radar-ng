import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";

// Server-side tile coverage caps per data product. Telling MapLibre the
// real max zoom lets it upsample from the highest available tile instead
// of fetching higher zooms, getting 404s, and painting nothing. Discovered
// the hard way: nowcast frames at z=7+ returned 404 and the overlay went
// blank during "Now" / next-hour playback. (Radar past frames render at
// z=9, nowcast pyramid only reaches z=6.)
const SOURCE_MAX_ZOOM: Record<string, number> = {
  radar: 9,
  "radar-hrrr": 9,
  nowcast: 6,
};

// Lowest zoom the tile pyramids actually render. Below this, MapLibre
// would fire 404-bound requests (e.g. /tiles/.../1/0/0.png) over the
// public Cloudflare hop, racking up ~250 ms RTT each and starving real
// tile fetches → "Failed to load tile 1/0/0=>1 ... timeout" in logs.
// MRMS coverage is CONUS-only, so very-low-zoom world tiles wouldn't be
// meaningful anyway; clamping here keeps the wire quiet.
const SOURCE_MIN_ZOOM = 4;

// One source per render. The earlier 7-frame preload pattern triggered an
// iOS NSRangeException inside `[MLRNMapView insertReactSubview:atIndex:]`
// — Fragment-wrapped multi-source children confuse maplibre-react-native
// v10's native subview indexing on iOS, and the radar tab crashed on
// mount. Smooth scrubbing is nice-to-have; not crashing is mandatory.
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
  const maxZoom = SOURCE_MAX_ZOOM[layerForUrl] ?? 9;

  return (
    <MapLibreGL.RasterSource
      id="radar-source"
      key={`${activePalette}-${layerForUrl}-${frame.path}`}
      tileUrlTemplates={[tileUrl]}
      tileSize={256}
      minZoomLevel={SOURCE_MIN_ZOOM}
      maxZoomLevel={maxZoom}
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
