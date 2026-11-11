import { replayClockLabel, replayFrameNote, replayTimeLabel, type ReplayFrame } from "./replaySource";
import type { RadarFrame } from "../types/weather";

const AXIS_FRACTIONS = [0, 0.25, 0.5, 0.75, 1];

/** Header text for a replay frame: absolute date, time and zone, never "Now" or an offset. */
export function replayTimelineHeader(frame: RadarFrame | undefined, layerTitle: string, timeZone: string) {
  if (!frame) return { title: `${layerTitle} · Historical`, dateLabel: "", note: "" };
  return {
    title: `${layerTitle} · Historical`,
    dateLabel: replayTimeLabel(frame.time, timeZone),
    note: replayFrameNote(frame as ReplayFrame, timeZone),
  };
}

/** Axis ticks as clock time + zone in the case's zone. */
export function replayAxisLabels(frames: readonly RadarFrame[], sequence: readonly number[], timeZone: string): string[] {
  if (sequence.length === 0) return [];
  const last = sequence.length - 1;
  return AXIS_FRACTIONS.map((f) => replayClockLabel(frames[sequence[Math.round(f * last)]].time, timeZone));
}
