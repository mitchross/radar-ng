import { Layer, RasterSource } from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";

// Server-side tile coverage caps per data product. Telling MapLibre the
// real max zoom lets it upsample from the highest available tile instead
// of fetching higher zooms, getting 404s, and painting nothing. Discovered
// the hard way: nowcast frames at z=7+ returned 404 and the overlay went
// blank during "Now" / next-hour playback.
//
// 2026-05-10: dropped radar+radar-hrrr from 9 to 8. Z9 alone was ~75% of
// the per-frame render wall-clock and the schedule cadence couldn't keep
// up (frames were ~15 min stale). Pinching past z=8 now upsamples the z=8
// tile, which softens detail but keeps the overlay aligned. Restore to 9
// when render perf catches up at the source.
const SOURCE_MAX_ZOOM: Record<string, number> = {
  radar: 8,
  "radar-hrrr": 8,
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
// — Fragment-wrapped multi-source children confused native subview indexing
// on iOS, and the radar tab crashed on mount. Smooth scrubbing is nice-to-have;
// not crashing is mandatory.
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
