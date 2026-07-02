import { Layer, RasterSource } from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import { assignSlots, clampWindow } from "../../lib/radarCarousel";

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

// Number of carousel slots. Playback swaps frames by flipping raster-opacity
// between pre-mounted sources (paint updates apply to live native layers
// without remount); each tick remounts exactly ONE hidden slot, giving it
// (WINDOW-1) × tick ≈ 1.7 s to fetch tiles before it is shown.
//
// iOS history: an earlier 7-frame preload crashed with an NSRangeException
// in [MLRNMapView insertReactSubview:atIndex:] because the child COUNT
// churned (sources conditionally mounted/unmounted inside a Fragment).
// The carousel keeps a constant child count — slots are a keyed array and
// only ever replace in place — which is the property that avoids the
// index desync. WINDOW = 1 reproduces the old single-source behavior and
// is the kill switch if a regression ever shows up on a real iPhone.
const WINDOW = 5;

export function RadarOverlay() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);

  if (frames.length === 0 || currentFrameIndex < 0) return null;
  const clampedIndex = Math.min(currentFrameIndex, frames.length - 1);

  const { start, end } = clampWindow(playbackWindow, frames.length);
  const { slots, visibleSlot } = assignSlots(clampedIndex, start, end, WINDOW);

  return slots.map((frameIndex, slot) => {
    // assignSlots only yields indices inside the clamped window, so this
    // fallback should never fire — it exists to keep the child COUNT
    // constant (a conditional null here is exactly the churn that caused
    // the historical iOS insertReactSubview crash).
    const frame = frames[frameIndex] ?? frames[clampedIndex];

    // In forecast mode the frame list is a merged radar + nowcast + HRRR
    // stream — per-frame `source` tells us which tile subtree to hit.
    const layerForUrl = frame.source ?? activeLayer;
    const tileUrl = buildSelfHostedTileUrl(serverUrl, layerForUrl, frame.path, activePalette);
    const maxZoom = SOURCE_MAX_ZOOM[layerForUrl] ?? 9;
    // Unique per assignment: satisfies useFrozenId (id may never change on
    // a mounted source) and sidesteps iOS reusing a stale native source
    // when a new one briefly shares the same id during replacement.
    const sourceId = `radar-src-${slot}-${activePalette}-${layerForUrl}-${frame.path}`;
    const isVisible = slot === visibleSlot;

    return (
      <RasterSource
        id={sourceId}
        key={sourceId}
        tiles={[tileUrl]}
        tileSize={256}
        minzoom={SOURCE_MIN_ZOOM}
        maxzoom={maxZoom}
      >
        <Layer
          type="raster"
          id={`radar-layer-${slot}-${activePalette}-${layerForUrl}-${frame.path}`}
          paint={{
            // Hidden slots keep visibility=visible at opacity 0 so MapLibre
            // still fetches their tiles into its own cache — an HTTP-level
            // prefetch would not reach MapLibre's resource loader.
            "raster-opacity": isVisible && radarVisible ? radarOpacity : 0,
            "raster-fade-duration": 0,
          }}
        />
      </RasterSource>
    );
  });
}
