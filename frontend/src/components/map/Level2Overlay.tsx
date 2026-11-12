import { useWeatherStore } from "../../stores/useWeatherStore";
import { level2TileTemplate } from "../../lib/level2";
import { RasterFrameCarousel, WEATHER_ANCHOR_ID, type CarouselFrameSpec } from "./RasterFrameCarousel";
import type { RadarFrame } from "../../types/weather";


/** The close-up sweeps over MRMS: same carousel, own tile subtree, faded in by zoom. */
export function Level2Overlay() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const { view, opacity } = useWeatherStore((s) => s.level2);

  const closeUp = view !== null && frames.length > 0 && frames.every((f) => f.level2);
  if (!view || !closeUp) return null;

  const specFor = (frame: RadarFrame): CarouselFrameSpec => ({
    tileUrl: level2TileTemplate(serverUrl, view, frame.level2!),
    maxZoom: view.sourceZoom,
    variant: `l2-${view.viewId}-${view.revision}`,
  });

  return (
    <RasterFrameCarousel
      idPrefix="level2"
      frames={frames}
      currentFrameIndex={currentFrameIndex}
      playbackWindow={playbackWindow}
      opacity={radarOpacity * opacity}
      minZoom={view.sourceZoom}
      specFor={specFor}
      beforeId={WEATHER_ANCHOR_ID}
    />
  );
}
