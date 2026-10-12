import { describe, expect, it } from "vitest";
import { buildFrames, nowIndex, offsetLabel, playbackSequence, refreshKeepTime, sameFrames, type Manifest } from "./frames";

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

describe("refreshKeepTime", () => {
  const frames = buildFrames(manifest, "radar", NOW);
  const live = frames[nowIndex(frames, NOW)];
  const scrubbed = frames[nowIndex(frames, NOW) - 5];
  it("holds the playhead while playing, even on the live frame", () => {
    expect(refreshKeepTime(live, live, true)).toBe(live.time);
    expect(refreshKeepTime(scrubbed, live, true)).toBe(scrubbed.time);
  });
  it("holds a scrubbed playhead and follows now only when parked on live", () => {
    expect(refreshKeepTime(scrubbed, live, false)).toBe(scrubbed.time);
    expect(refreshKeepTime(live, live, false)).toBeUndefined();
    expect(refreshKeepTime(undefined, live, false)).toBeUndefined();
  });
});

describe("buildFrames observed product", () => {
  it("uses the composite for past frames when asked, falling back to base", () => {
    const withComposite: Manifest = {
      layers: { ...manifest.layers, "radar-composite": { frames: [{ timestamp: iso(NOW - 60), path: "c0" }] } },
    };
    expect(buildFrames(withComposite, "radar", NOW, "radar-composite").filter((f) => f.time <= NOW).map((f) => f.source)).toEqual(["radar-composite"]);
    expect(buildFrames(manifest, "radar", NOW, "radar-composite").some((f) => f.source === "radar")).toBe(true);
  });
});

describe("sameFrames", () => {
  it("treats a rebuild from an unchanged manifest as the same timeline", () => {
    const a = buildFrames(manifest, "radar", NOW);
    const b = buildFrames(manifest, "radar", NOW);
    expect(a).not.toBe(b);
    expect(sameFrames(a, b)).toBe(true);
  });

  it("sees a new frame, a moved path or a new source", () => {
    const a = buildFrames(manifest, "radar", NOW);
    expect(sameFrames(a, a.slice(1))).toBe(false);
    expect(sameFrames(a, a.map((f, i) => (i === 3 ? { ...f, path: "other" } : f)))).toBe(false);
    expect(sameFrames(a, a.map((f, i) => (i === 3 ? { ...f, source: "radar-composite" } : f)))).toBe(false);
  });
});
