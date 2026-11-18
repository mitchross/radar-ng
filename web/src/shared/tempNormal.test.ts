import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import contract from "./testdata/normals.contract.json";
import type { ColorRange } from "./serverPalettes";
import {
  TEMP_NORMAL_LAYER,
  anomalyWords,
  formatAnomalyReading,
  inLower48,
  normalsLine,
  normalsPath,
  parseNormals,
  tempAnomalyLegend,
} from "./tempNormal";

const palette = (id: string) =>
  JSON.parse(readFileSync(new URL(`../../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url), "utf8")) as Record<string, { ranges: ColorRange[] }>;

describe("temp vs normal", () => {
  it("parses the server contract", () => {
    expect(parseNormals(contract.detroit)).toEqual({ date: "2026-11-18", highF: 46.9, lowF: 30.9 });
    expect(parseNormals(contract.phoenix_july)?.highF).toBe(106.7);
    expect(parseNormals({ error: "normals_not_ready" })).toBeNull();
    expect(parseNormals(null)).toBeNull();
  });

  it("compares the forecast high with the normal high", () => {
    const n = parseNormals(contract.detroit);
    expect(normalsLine(58.4, n)).toBe("High 58\u00B0, 12\u00B0 above normal");
    expect(normalsLine(40, n)).toBe("High 40\u00B0, 7\u00B0 below normal");
    expect(normalsLine(47.2, n)).toBe("High 47\u00B0, near normal");
    expect(normalsLine(null, n)).toBeNull();
    expect(normalsLine(58, null)).toBeNull();
    expect(normalsLine(58.4, n, true)).toBe("High 15\u00B0, 6\u00B0 above normal");
  });

  it("words a map value", () => {
    expect(formatAnomalyReading(-3.4)).toBe("3\u00B0 below normal");
    expect(formatAnomalyReading(Number.NaN)).toBeNull();
    expect(anomalyWords(0.4)).toBe("near normal");
  });

  it("is lower 48 only", () => {
    expect(inLower48(42.33, -83.05)).toBe(true);
    expect(inLower48(61.2, -149.9)).toBe(false);
    expect(inLower48(21.3, -157.8)).toBe(false);
    expect(normalsPath(42.333, -83.05, "2026-11-18")).toBe("/api/normals/42.33/-83.05?date=2026-11-18");
  });

  it("builds the legend from every server palette; the fallback is classic", () => {
    for (const id of ["classic", "muted", "vivid"]) {
      const ranges = palette(id)[TEMP_NORMAL_LAYER.paletteKey].ranges;
      expect(ranges[0].min).toBeLessThan(-20);
      expect(ranges[ranges.length - 1].max).toBeGreaterThan(20);
      const stops = tempAnomalyLegend(ranges);
      expect(stops.map((s) => s.label)).toEqual(["-20\u00B0", "-10\u00B0", "Normal", "+10\u00B0", "+20\u00B0"]);
      expect(new Set(stops.map((s) => s.color)).size).toBe(5);
    }
    expect(tempAnomalyLegend(null)).toEqual(tempAnomalyLegend(palette("classic").temp_anomaly.ranges));
  });
});
