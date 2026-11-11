import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { ReplayWarningHistory } from "./replayWarnings";
import {
  fullCaseSequence, isReplaySource, lookaheadPositions, LIVE_SOURCE, replayClockLabel, replayFrameNote, replayFrames,
  replayTileUrl, replayTimeLabel, replayWarningCollection, replayWarningCopy, replayWarningKey, sameSource, SourceGuard,
  sourceKey, type ReplayManifest,
} from "./replaySource";

const fixture: { history: ReplayWarningHistory } = JSON.parse(readFileSync(new URL("./replayWarnings.fixture.json", import.meta.url), "utf8"));
const at = (min: number) => new Date(Date.UTC(2023, 2, 25, 0, min)).toISOString();
const manifest: ReplayManifest = {
  schema_version: 2,
  tile_url_template: "/replays/c/r/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
  layers: {
    radar: { frames: [0, 2, 4, 6, 8, 10].map((m) => ({ timestamp: at(m), path: `r${m}` })) },
    "radar-composite": { frames: [0, 2, 4, 6, 8, 10].map((m) => ({ timestamp: at(m), path: `c${m}`, max_zoom: 6 })) },
    "hail-size": { frames: [{ timestamp: at(0), path: "h0" }, { timestamp: at(8), path: "h8" }] },
    "precip-24h": { frames: [{ timestamp: at(0), path: "p0" }] },
  },
};

describe("source context", () => {
  it("keys sources apart", () => {
    const a = { kind: "replay", caseId: "x", revision: "1" } as const;
    expect(sourceKey(LIVE_SOURCE)).toBe("live");
    expect(sourceKey(a)).toBe("replay:x:1");
    expect(sameSource(a, { ...a, revision: "2" })).toBe(false);
    expect(isReplaySource(a)).toBe(true);
    expect(isReplaySource(LIVE_SOURCE)).toBe(false);
  });
  it("aborts stale tickets on cancel", () => {
    const g = new SourceGuard();
    const old = g.ticket();
    expect(old.current()).toBe(true);
    g.cancel();
    expect(old.current()).toBe(false);
    expect(old.signal.aborted).toBe(true);
    expect(g.ticket().current()).toBe(true);
  });
});

describe("replay frames", () => {
  it("keeps every base frame, unthinned and unmerged", () => {
    const f = replayFrames(manifest, "radar");
    expect(f.map((x) => x.path)).toEqual(["r0", "r2", "r4", "r6", "r8", "r10"]);
    expect(f.every((x) => !x.gap && x.source === "radar")).toBe(true);
    expect(fullCaseSequence(f.length)).toEqual([0, 1, 2, 3, 4, 5]);
  });
  it("uses the layer's latest frame at or before the playhead, with a gap past the cap", () => {
    const f = replayFrames(manifest, "hail-size");
    expect(f.map((x) => x.path)).toEqual(["h0", "h0", "h0", "h0", "h8", "h8"]);
    expect(f.map((x) => Boolean(x.gap))).toEqual([false, false, false, true, false, false]);
    const sparse = replayFrames({ ...manifest, layers: { ...manifest.layers, "hail-size": { frames: [{ timestamp: at(0), path: "h0" }] } } }, "hail-size");
    expect(sparse.map((x) => Boolean(x.gap))).toEqual([false, false, false, true, true, true]);
  });
  it("lets hourly rainfall lag 90 minutes", () => {
    expect(replayFrames(manifest, "precip-24h").every((x) => !x.gap)).toBe(true);
  });
  it("honours manifest max zoom and returns nothing for a missing layer", () => {
    expect(replayFrames(manifest, "radar-composite")[0].maxZoom).toBe(6);
    expect(replayFrames(manifest, "echo-top")).toEqual([]);
  });
});

describe("prefetch and urls", () => {
  it("bounds lookahead and wraps at the loop end", () => {
    expect(lookaheadPositions(4, 6, 3)).toEqual([4, 5, 0, 1]);
    expect(lookaheadPositions(0, 2, 6)).toEqual([0, 1]);
    expect(lookaheadPositions(0, 0, 6)).toEqual([]);
  });
  it("fills the tile template", () => {
    expect(replayTileUrl(manifest.tile_url_template!, "radar", "classic", "a b/c")).toBe("/replays/c/r/tiles/radar/classic/a%20b/c/{z}/{x}/{y}.png");
  });
});

describe("labels", () => {
  it("shows absolute date, time and zone", () => {
    const sec = Date.UTC(2023, 2, 25, 0, 57) / 1000;
    expect(replayTimeLabel(sec, "America/Chicago")).toBe("Fri, Mar 24, 2023, 7:57 PM CDT");
    expect(replayClockLabel(sec, "America/Chicago")).toBe("7:57 PM CDT");
    expect(replayTimeLabel(sec, "America/Chicago")).not.toMatch(/now/i);
  });
  it("explains gaps and lagging layers", () => {
    const base = { time: 100 * 60 + Date.UTC(2023, 2, 25) / 1000, timestamp: at(0), path: "p", source: "hail-size", maxZoom: 7 };
    expect(replayFrameNote({ ...base, gap: true }, "UTC")).toMatch(/No data/);
    expect(replayFrameNote(base, "UTC")).toMatch(/^Latest data from/);
    expect(replayFrameNote({ ...base, time: Date.parse(at(0)) / 1000 }, "UTC")).toBe("");
    expect(replayFrameNote({ ...base, source: "precip-24h" }, "UTC")).toMatch(/Rolling 24-hour/);
  });
});

describe("warnings at the playhead", () => {
  const h = fixture.history;
  it("matches the shared fixture selection and keys changes", () => {
    const t = Date.parse("2023-03-25T00:10:00Z");
    const fc = replayWarningCollection(h, t);
    expect(fc.type).toBe("FeatureCollection");
    expect(fc.features.length).toBeGreaterThan(0);
    expect(replayWarningKey(fc)).toBe(fc.features.map((f) => f.id).join("|"));
    expect(replayWarningCollection(h, Date.parse("2023-03-26T00:00:00Z")).features).toEqual([]);
    expect(replayWarningCollection(null, t).features).toEqual([]);
  });
  it("words the popup as historical", () => {
    const fc = replayWarningCollection(h, Date.parse("2023-03-25T00:10:00Z"));
    const copy = replayWarningCopy(fc.features[0].properties, "America/Chicago");
    expect(copy.lines.at(-1)).toMatch(/Historical warning/);
    expect(copy.title.length).toBeGreaterThan(0);
  });
});
