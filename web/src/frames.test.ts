import { describe, expect, it } from "vitest";
import { buildFrames, nowIndex, offsetLabel, playbackSequence, type Manifest } from "./frames";

const NOW = 1_800_000_000;
const iso = (s: number) => new Date(s * 1000).toISOString();
const manifest: Manifest = {
  layers: {
    radar: { frames: Array.from({ length: 121 }, (_, i) => ({ timestamp: iso(NOW - (120 - i) * 120), path: `r${i}` })) },
    nowcast: { frames: Array.from({ length: 12 }, (_, i) => ({ timestamp: iso(NOW + (i + 1) * 300), path: `n${i}` })) },
    "radar-hrrr": { frames: Array.from({ length: 47 }, (_, i) => ({ timestamp: iso(NOW + (i + 1) * 3600), path: `h${i}` })) },
  },
};

describe("buildFrames", () => {
  it("merges observed, nowcast (to +60m) and HRRR (beyond) in time order", () => {
    const frames = buildFrames(manifest, "radar", NOW);
    expect(frames.filter((f) => f.source === "nowcast")).toHaveLength(12);
    // HRRR's +1h frame is covered by the nowcast window.
    expect(frames.filter((f) => f.source === "radar-hrrr")).toHaveLength(46);
    for (let i = 1; i < frames.length; i++) expect(frames[i].time).toBeGreaterThan(frames[i - 1].time);
    expect(frames[nowIndex(frames, NOW)].time).toBe(NOW);
  });
});

describe("playbackSequence", () => {
  it("makes the 48h loop mostly future", () => {
    const frames = buildFrames(manifest, "radar", NOW);
    const seq = playbackSequence(frames, "48h", NOW);
    const future = seq.filter((i) => frames[i].time > NOW).length;
    expect(future).toBe(58);
    expect(seq.length - future).toBeLessThanOrEqual(17);
  });
  it("keeps 1h to an hour either side", () => {
    const frames = buildFrames(manifest, "radar", NOW);
    const seq = playbackSequence(frames, "1h", NOW);
    for (const i of seq) expect(Math.abs(frames[i].time - NOW)).toBeLessThanOrEqual(3600);
  });
});

describe("offsetLabel", () => {
  it("formats offsets", () => {
    expect(offsetLabel(NOW + 120, NOW)).toBe("Now");
    expect(offsetLabel(NOW + 45 * 60, NOW)).toBe("+45m");
    expect(offsetLabel(NOW - 4 * 3600, NOW)).toBe("−4h");
  });
});
