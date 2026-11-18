import { describe, expect, it } from "vitest";
import { NICE_DAY_LAYER, formatNiceDayReading, niceDayLegend } from "./niceDayLayer";
import { LAYER_PALETTE_KEY } from "./serverPalettes";
import { readFileSync } from "node:fs";
import type { ColorRange } from "./serverPalettes";

const palette = (id: string) =>
  JSON.parse(readFileSync(new URL(`../../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url), "utf8")) as Record<string, { ranges: ColorRange[] }>;

describe("nice-day map layer", () => {
  it("has five steps, best first, one per server band in every palette", () => {
    for (const id of ["classic", "muted", "vivid"]) {
      const ranges = palette(id).nice_day.ranges;
      expect(ranges.map((r) => r.min)).toEqual([0, 20, 40, 60, 80]);
      const stops = niceDayLegend(ranges);
      expect(stops.map((s) => s.word)).toEqual(["Great", "Good", "Fair", "Poor", "Do Not"]);
      expect(new Set(stops.map((s) => s.color)).size).toBe(5);
    }
  });

  it("falls back to fixed colors before the palettes load", () => {
    expect(niceDayLegend(null)).toHaveLength(5);
    expect(niceDayLegend(null)[0].color).toBe("#32aa5a");
  });

  it("reads a value as its word and score", () => {
    expect(formatNiceDayReading(100)).toBe("Great · 100");
    expect(formatNiceDayReading(72.4)).toBe("Good · 72");
    expect(formatNiceDayReading(40)).toBe("Fair · 40");
    expect(formatNiceDayReading(19.6)).toBe("Poor · 20");
    expect(formatNiceDayReading(0)).toBe("Do Not · 0");
    expect(formatNiceDayReading(null)).toBeNull();
    expect(formatNiceDayReading(Number.NaN)).toBeNull();
  });

  it("maps the layer to its palette key", () => {
    expect(LAYER_PALETTE_KEY[NICE_DAY_LAYER.id]).toBe(NICE_DAY_LAYER.paletteKey);
  });
});
