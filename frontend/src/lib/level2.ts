import { closeUpFrames, level2TileUrl, type Level2Frame, type Level2View } from "../../../web/src/shared/level2";
import type { RadarFrame } from "../types/weather";

// Selection, age, fade, loop, height and wording live in one module shared with the web radar.
export * from "../../../web/src/shared/level2";

/** The last six Level II sweeps as timeline frames, each over its MRMS frame or an explicit gap. */
export function closeUpRadarFrames(view: Level2View, observed: readonly RadarFrame[]): RadarFrame[] {
  return closeUpFrames(view, observed).map((c): RadarFrame =>
    c.gap
      ? { time: c.time, timestamp: c.level2.timestamp, path: "", source: "radar", gap: true, level2: c.level2 }
      : { ...observed[c.mrmsIndex], time: c.time, level2: c.level2 },
  );
}


/** Tile URL (with {z}/{x}/{y}) for one sweep on our own server. */
export const level2TileTemplate = (serverUrl: string, view: Level2View, frame: Level2Frame): string => `${serverUrl}${level2TileUrl(view, frame)}`;
