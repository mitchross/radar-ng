import { useEffect, useRef } from "react";
import { useWeatherStore } from "../stores/useWeatherStore";
import { clampWindow } from "../lib/radarCarousel";
import { flushPlaybackMetrics, markPlaybackTick } from "../lib/playbackMetrics";

const PLAYBACK_MS = 420;

/**
 * Renderless playback ticker — advances `currentFrameIndex` through the
 * active playback window every 420 ms while `isPlaying`.
 *
 * Lives in its own hook (called once from the radar screen) so the timer
 * isn't owned by a visual component: TimelineBar re-rendering, remounting,
 * or being restyled can't duplicate or drop the loop.
 */
export function usePlaybackTicker() {
  const isPlaying = useWeatherStore((s) => s.isPlaying);
  const framesLength = useWeatherStore((s) => s.frames.length);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);

  // Read the index through a ref so the interval doesn't restart per tick.
  const idxRef = useRef(useWeatherStore.getState().currentFrameIndex);
  useEffect(
    () =>
      useWeatherStore.subscribe((s) => {
        idxRef.current = s.currentFrameIndex;
      }),
    [],
  );

  const { start, end } = clampWindow(playbackWindow, framesLength);

  useEffect(() => {
    if (!isPlaying || framesLength === 0 || end <= start) return;
    const id = setInterval(() => {
      const next = idxRef.current + 1 > end || idxRef.current + 1 < start
        ? start
        : idxRef.current + 1;
      markPlaybackTick();
      useWeatherStore.getState().setCurrentFrameIndex(next);
    }, PLAYBACK_MS);
    return () => {
      clearInterval(id);
      // One summary telemetry event per play session (tick→render p50/p95).
      flushPlaybackMetrics();
    };
  }, [isPlaying, start, end, framesLength]);
}
