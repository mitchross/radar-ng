import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import { RasterFrameCarousel, type CarouselFrameSpec } from "./RasterFrameCarousel";
import type { RadarFrame } from "../../types/weather";

// Real pyramid ceiling per product (MRMS 7, nowcast/HRRR 6); a manifest
// `max_zoom` overrides. Overstating it makes MapLibre fetch 404 tiles and paint nothing.
const SOURCE_MAX_ZOOM: Record<string, number> = {
  radar: 7,
  "radar-hrrr": 6,
  nowcast: 6,
};

// Pyramids start at z4 (CONUS only); lower zooms are 404s that starve real tile fetches.
const SOURCE_MIN_ZOOM = 4;

// Slot count + swap mechanics live in RasterFrameCarousel / lib/radarCarousel (CAROUSEL_WINDOW).
export function RadarOverlay() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);

  const specFor = (frame: RadarFrame): CarouselFrameSpec => {
    // In forecast mode the frame list is a merged radar + nowcast + HRRR stream —
    // per-frame `source` tells us which tile subtree to hit.
    const layerForUrl = frame.source ?? activeLayer;
    return {
      tileUrl: buildSelfHostedTileUrl(serverUrl, layerForUrl, frame.path, activePalette),
      maxZoom: frame.maxZoom ?? SOURCE_MAX_ZOOM[layerForUrl] ?? 7,
      variant: `${activePalette}-${layerForUrl}`,
    };
  };

  return (
    <RasterFrameCarousel
      idPrefix="radar"
      frames={frames}
      currentFrameIndex={currentFrameIndex}
      playbackWindow={playbackWindow}
      opacity={radarVisible ? radarOpacity : 0}
      minZoom={SOURCE_MIN_ZOOM}
      specFor={specFor}
    />
  );
}
