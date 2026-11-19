import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { ECCC_CREDIT, STARGAZING_LAYER, bestWindowLine, formatStargazingReading, parseSkyQuality, parseStargazing, skyQualityLine, skyQualityNote, stargazingLegend, stargazingWord } from "./stargazingLayer";
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

describe("ECCC seeing and transparency", () => {
  it("reads only whole 0-5 classes and names them as ECCC does", () => {
    expect(parseSkyQuality({ seeing: 3, transparency: 4 })).toEqual({ seeing: 3, transparency: 4 });
    expect(parseSkyQuality({ seeing: 3 })).toBeNull();
    expect(parseSkyQuality({ seeing: 6, transparency: 4 })).toBeNull();
    expect(parseSkyQuality({ seeing: 2.5, transparency: 4 })).toBeNull();
    expect(parseSkyQuality(null)).toBeNull();
    expect(skyQualityLine({ seeing: 3, transparency: 4 })).toBe("Seeing Average · Transparency Good");
    expect(skyQualityLine({ seeing: 0, transparency: 0 })).toBe("Seeing and transparency: cloudy");
    expect(skyQualityLine(null)).toBeNull();
  });

  it("credits ECCC in the inspector note and the legend", () => {
    expect(skyQualityNote({ ok: true, value: 62, seeing: 5, transparency: 1 })).toBe(
      "Seeing Excellent · Transparency Very bad · Data Source: Environment and Climate Change Canada",
    );
    expect(skyQualityNote({ ok: true, value: 62 })).toBeNull();
    expect(ECCC_CREDIT).toBe("Data Source: Environment and Climate Change Canada");
    expect(STARGAZING_LAYER.seeingCredit).toContain(ECCC_CREDIT);
  });

  it("takes the best window's hour, else the first hour that has values", () => {
    const hours = [
      { at: "2026-11-18T02:00:00Z", score: 40, seeing: 2, transparency: 2 },
      { at: "2026-11-18T03:00:00Z", score: 84, seeing: 4, transparency: 5 },
      { at: "2026-11-18T04:00:00Z", score: 80 },
    ];
    const best = { start: "2026-11-18T03:00:00Z", end: "2026-11-18T05:00:00Z", score: 84 };
    expect(parseStargazing({ hours, best })?.quality).toEqual({ seeing: 4, transparency: 5 });
    expect(parseStargazing({ hours, best: null })?.quality).toEqual({ seeing: 2, transparency: 2 });
    expect(parseStargazing({ hours: [{ at: "2026-11-18T03:00:00Z", score: 84 }], best })).toEqual({ best: { start: at(best.start), end: at(best.end), score: 84 } });
  });

  it("shows the line and the credit on the card only when the server has them", () => {
    const now = at("2026-11-17T23:00:00Z");
    const night = tonight(now, "2026-11-17", 43.65, -79.38)!;
    const base = { nowMs: now, night, lat: 43.65, lon: -79.38, hourly: null, clock };
    const best = { start: "2026-11-18T03:00:00Z", end: "2026-11-18T07:00:00Z", score: 84 };
    const withQ = parseSkyTonight({
      status: "ok",
      passes: [],
      stargazing: { hours: [{ at: best.start, score: 84, seeing: 3, transparency: 4 }], best, seeing_source: ECCC_CREDIT },
    });
    const card = skyTonightCard({ ...base, report: withQ });
    expect(card.skyQuality).toBe("Seeing Average · Transparency Good");
    expect(card.seeingCredit).toBe(ECCC_CREDIT);
    const plain = skyTonightCard({ ...base, report: parseSkyTonight({ status: "ok", passes: [], stargazing: { hours: [], best } }) });
    expect(plain.skyQuality).toBeNull();
    expect(plain.seeingCredit).toBeNull();
    expect(plain.bestWindow).toBe("Best 10 PM – 2 AM · Excellent");
  });
});
