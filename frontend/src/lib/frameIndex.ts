// Nearest-frame lookup for the radar timeline. Pure (no RN deps) so it can be
// unit-tested directly — see __tests__/lib/frameIndex.test.ts.

/**
 * Index of the frame whose `time` is closest to `target`.
 *
 * Guards empty input: TimelineBar's segment-boundary useMemo runs before the
 * component's `frames.length === 0` early return (rules-of-hooks), so this is
 * called with no frames on first load. Without the guard, `frames[-1].time`
 * throws "Cannot read property 'time' of undefined" and the Radar screen
 * render-errors before any frames arrive.
 */
export function findClosestIdx(frames: { time: number }[], target: number): number {
  if (frames.length === 0) return 0;
  let best = 0;
  let bestDiff = Infinity;
  for (let i = 0; i < frames.length; i++) {
    const d = Math.abs(frames[i].time - target);
    if (d < bestDiff) {
      bestDiff = d;
      best = i;
    }
  }
  if (target > frames[frames.length - 1].time) return frames.length - 1;
  return best;
}

/**
 * Where the frame the user was looking at (by wall-clock `prevTime`) lives in a
 * refreshed frame list. Indices shift every manifest poll (head pruned, run
 * rolled), so a bare index would silently move the paused frame forward in time.
 *
 * Returns -1 when the caller should snap to "now" instead: empty list, an
 * explicit -1 index, or no frame within `toleranceSec` of `prevTime` (layer switch).
 */
export function resnapFrameIndex(
  frames: { time: number }[],
  currentIndex: number,
  prevTime: number | null,
  toleranceSec = 15 * 60,
): number {
  if (frames.length === 0 || currentIndex < 0) return -1;
  if (prevTime === null) return currentIndex < frames.length ? currentIndex : -1;
  if (frames[currentIndex]?.time === prevTime) return currentIndex;
  const idx = findClosestIdx(frames, prevTime);
  return Math.abs(frames[idx].time - prevTime) <= toleranceSec ? idx : -1;
}
