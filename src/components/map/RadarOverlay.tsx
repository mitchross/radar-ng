import MapLibreGL from "@maplibre/maplibre-react-native";
import { useMemo } from "react";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";

// How many frames to keep mounted on each side of the current index. With
// 3 + 3 = 7 sources warm at any time, scrubbing the timeline or playing
// the loop swaps tiles instantly instead of unmount → re-fetch → render.
// Apple Weather / RainViewer use the same pattern.
const PRELOAD_RADIUS = 3;

export function RadarOverlay() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const activePalette = useWeatherStore((s) => s.activePalette);

  // Window of frames around the current index. Memoized on the active key
  // bits so we don't churn sources for opacity-only updates.
  const windowed = useMemo(() => {
    if (frames.length === 0 || currentFrameIndex < 0) return [];
    const start = Math.max(0, currentFrameIndex - PRELOAD_RADIUS);
    const end = Math.min(frames.length, currentFrameIndex + PRELOAD_RADIUS + 1);
    return frames.slice(start, end).map((f, i) => ({
      frame: f,
      absoluteIndex: start + i,
    }));
  }, [frames, currentFrameIndex]);

  if (windowed.length === 0) return null;

  return (
    <>
      {windowed.map(({ frame, absoluteIndex }) => {
        const layerForUrl = frame.source ?? activeLayer;
        const tileUrl = buildSelfHostedTileUrl(
          serverUrl,
          layerForUrl,
          frame.path,
          activePalette,
        );
        const isCurrent = absoluteIndex === currentFrameIndex;
        return (
          <MapLibreGL.RasterSource
            id={`radar-${frame.path}`}
            key={`${activePalette}-${layerForUrl}-${frame.path}`}
            tileUrlTemplates={[tileUrl]}
            tileSize={256}
            minZoomLevel={1}
            maxZoomLevel={12}
          >
            <MapLibreGL.RasterLayer
              id={`radar-layer-${frame.path}`}
              style={{
                rasterOpacity: isCurrent && radarVisible ? radarOpacity : 0,
                // Short fade smooths the swap between adjacent frames.
                rasterFadeDuration: 120,
                // Linear resampling — display-side smoothing on top of the
                // backend-rendered PNG. Cheap, looks much crisper than the
                // default nearest snap.
                rasterResampling: "linear",
              }}
            />
          </MapLibreGL.RasterSource>
        );
      })}
    </>
  );
}
