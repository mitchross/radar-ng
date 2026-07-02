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
