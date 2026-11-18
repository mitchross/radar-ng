import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { STARGAZING_LAYER, bestWindowLine, formatStargazingReading, parseStargazing, stargazingLegend, stargazingWord } from "./stargazingLayer";
import { LAYER_PALETTE_KEY, type ColorRange } from "./serverPalettes";
import { parseSkyTonight, skyTonightCard, tonight } from "./skyTonight";

const palette = (id: string) =>
  JSON.parse(readFileSync(new URL(`../../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url), "utf8")) as Record<string, { ranges: ColorRange[] }>;

const clock = { timeZone: "America/Detroit", utcOffsetSeconds: -5 * 3600 };
const at = (iso: string) => Date.parse(iso);

describe("stargazing map layer", () => {
  it("has four steps, best first, one per server band in every palette", () => {
    for (const id of ["classic", "muted", "vivid"]) {
      const ranges = palette(id).stargazing.ranges;
      expect(ranges.map((r) => r.min)).toEqual([0, 25, 50, 75]);
      const stops = stargazingLegend(ranges);
      expect(stops.map((s) => s.word)).toEqual(["Excellent", "Good", "Fair", "Poor"]);
      expect(new Set(stops.map((s) => s.color)).size).toBe(4);
    }
  });

  it("falls back to the classic colors before the palettes load", () => {
    const classic = stargazingLegend(palette("classic").stargazing.ranges);
    expect(stargazingLegend(null)).toEqual(classic);
  });

  it("reads a value as its word and score", () => {
    expect(formatStargazingReading(100)).toBe("Excellent · 100");
    expect(formatStargazingReading(62.4)).toBe("Good · 62");
    expect(formatStargazingReading(25)).toBe("Fair · 25");
    expect(formatStargazingReading(3)).toBe("Poor · 3");
    expect(formatStargazingReading(null)).toBeNull();
    expect(formatStargazingReading(Number.NaN)).toBeNull();
    expect(stargazingWord(74.9)).toBe("Good");
  });

  it("maps the layer to its palette key", () => {
    expect(LAYER_PALETTE_KEY[STARGAZING_LAYER.id]).toBe(STARGAZING_LAYER.paletteKey);
  });
});

describe("best stargazing window", () => {
  const w = { start: at("2026-11-18T03:00:00Z"), end: at("2026-11-18T07:00:00Z"), score: 84 };

  it("reads as local clock times and a word", () => {
    expect(bestWindowLine(w, at("2026-11-17T23:00:00Z"), clock)).toBe("Best 10 PM – 2 AM · Excellent");
    expect(bestWindowLine({ ...w, end: at("2026-11-18T06:30:00Z") }, at("2026-11-17T23:00:00Z"), clock)).toBe("Best 10 PM – 1:30 AM · Excellent");
  });

  it("says until once it has begun, and nothing after", () => {
    expect(bestWindowLine(w, at("2026-11-18T04:10:00Z"), clock)).toBe("Best until 2 AM · Excellent");
    expect(bestWindowLine(w, at("2026-11-18T07:00:00Z"), clock)).toBeNull();
    expect(bestWindowLine(null, 0, clock)).toBeNull();
  });

  it("parses the server block and rejects bad windows", () => {
    expect(parseStargazing(undefined)).toBeNull();
    expect(parseStargazing({ layer: "stargazing", hours: [], best: null })).toEqual({ best: null });
    expect(parseStargazing({ best: { start: "2026-11-18T07:00:00Z", end: "2026-11-18T03:00:00Z", score: 80 } })).toEqual({ best: null });
    expect(parseStargazing({ best: { start: "2026-11-18T03:00:00Z", end: "2026-11-18T07:00:00Z", score: 84 } })).toEqual({ best: w });
  });
});

describe("Look up tonight with the star map", () => {
  const now = at("2026-11-17T23:00:00Z");
  const night = tonight(now, "2026-11-17", 42.3314, -83.0458)!;
  const base = { nowMs: now, night, lat: 42.3314, lon: -83.0458, hourly: null, clock };
  const body = { status: "ok", passes: [] };

  it("adds the best window and the map link when the layer is published", () => {
    const report = parseSkyTonight({ ...body, stargazing: { best: { start: "2026-11-18T03:00:00Z", end: "2026-11-18T07:00:00Z", score: 84 } } });
    const card = skyTonightCard({ ...base, report });
    expect(card.bestWindow).toBe("Best 10 PM – 2 AM · Excellent");
    expect(card.starMap).toBe(true);
  });

  it("links the map with no line when tonight has no good window", () => {
    const card = skyTonightCard({ ...base, report: parseSkyTonight({ ...body, stargazing: { best: null } }) });
    expect(card.bestWindow).toBeNull();
    expect(card.starMap).toBe(true);
  });

  it("shows neither when the server has no stargazing layer", () => {
    const report = parseSkyTonight(body);
    expect(report && "stargazing" in report).toBe(false);
    const card = skyTonightCard({ ...base, report });
    expect(card.bestWindow).toBeNull();
    expect(card.starMap).toBe(false);
  });
});
