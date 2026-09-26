/**
 * Which frames playback visits for each timeline zoom, in order.
 *
 * The frame list is dense in the past (MRMS every ~2 min), finer near now
 * (nowcast every 5 min) and hourly in the future (HRRR). Stepping every index
 * made the 48h loop mostly past radar. The 48h sequence thins the past to
 * one frame per PAST_STEP_48H and keeps every future frame, so the loop is
 * about what's coming.
 */
export type TimelineZoom = "1h" | "48h";

export interface TimedFrame {
  /** Epoch seconds. */
  time: number;
}

const HOUR = 60 * 60;
const PAST_STEP_48H = 15 * 60;

export function playbackSequence(frames: readonly TimedFrame[], zoom: TimelineZoom, nowSec: number): number[] {
  if (frames.length === 0) return [];
  if (zoom === "1h") {
    const seq: number[] = [];
    frames.forEach((f, i) => {
      if (f.time >= nowSec - HOUR && f.time <= nowSec + HOUR) seq.push(i);
    });
    return seq.length > 0 ? seq : [nearestIndex(frames, nowSec)];
  }
  // 48h: walk the past backwards from the latest past frame so "now" is kept,
  // taking one frame per PAST_STEP_48H; keep every future frame.
  const past: number[] = [];
  let lastKept = Infinity;
  for (let i = frames.length - 1; i >= 0; i--) {
    const t = frames[i].time;
    if (t > nowSec) continue;
    if (lastKept - t >= PAST_STEP_48H) {
      past.push(i);
      lastKept = t;
    }
  }
  past.reverse();
  const future: number[] = [];
  frames.forEach((f, i) => {
    if (f.time > nowSec) future.push(i);
  });
  return [...past, ...future];
}

/** Index into `frames` of the frame closest in time to `sec`. */
export function nearestIndex(frames: readonly TimedFrame[], sec: number): number {
  let best = 0;
  for (let i = 1; i < frames.length; i++) {
    if (Math.abs(frames[i].time - sec) < Math.abs(frames[best].time - sec)) best = i;
  }
  return best;
}

/** Position in `seq` of the sequence frame closest in time to frame `index`. */
export function positionOf(seq: readonly number[], frames: readonly TimedFrame[], index: number): number {
  const exact = seq.indexOf(index);
  if (exact >= 0 || seq.length === 0) return Math.max(0, exact);
  const t = frames[index]?.time ?? 0;
  let best = 0;
  for (let p = 1; p < seq.length; p++) {
    if (Math.abs(frames[seq[p]].time - t) < Math.abs(frames[seq[best]].time - t)) best = p;
  }
  return best;
}

/** "-4h", "+45m", "+17h": a frame's offset from now for axis labels. */
export function offsetLabel(frameSec: number, nowSec: number): string {
  const min = Math.round((frameSec - nowSec) / 60);
  if (Math.abs(min) < 1) return "Now";
  const sign = min > 0 ? "+" : "−";
  const abs = Math.abs(min);
  return abs < 90 ? `${sign}${abs}m` : `${sign}${Math.round(abs / 60)}h`;
}
