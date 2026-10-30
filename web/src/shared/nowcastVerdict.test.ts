import { describe, expect, it } from "vitest";
import { describeNowcast, getNowcastVerdict, interpolateNowcast } from "./nowcastVerdict";
import { RAIN_START_IN_HR } from "./rainAlerts";

describe("shared nowcast verdict", () => {
  it("is unavailable without values and dry or light below the rain threshold", () => {
    expect(getNowcastVerdict(undefined)).toEqual({ kind: "unavailable" });
    expect(getNowcastVerdict(Array(60).fill(0))).toEqual({ kind: "dry" });
    expect(getNowcastVerdict(Array(60).fill(RAIN_START_IN_HR))).toEqual({ kind: "light" });
  });

  it("starts rain strictly above RAIN_START_IN_HR and ends at the first minute above the end threshold", () => {
    const starting = Array(60).fill(0).map((_, i) => (i >= 20 && i < 40 ? (i === 30 ? 0.5 : 0.2) : 0));
    expect(getNowcastVerdict(starting)).toEqual({ kind: "starting", startMinute: 20, peakMinute: 30, endMinute: 39 });
    expect(getNowcastVerdict(Array(60).fill(0.2))).toEqual({ kind: "raining", peakMinute: 0, endMinute: 59 });
  });

  it("words each verdict the same on web and app", () => {
    expect(describeNowcast({ kind: "raining", peakMinute: 3, endMinute: 40 })).toBe(
      "Raining now, peaks at 3 minutes, and ends near 40 minutes.",
    );
  });

  it("interpolates lead minutes to a 60-minute inches/hour series and ignores points without a lead", () => {
    const v = interpolateNowcast([
      { lead_minutes: null, precipitation_mm_h: 99 },
      { lead_minutes: 5, precipitation_mm_h: 0 },
      { lead_minutes: 10, precipitation_mm_h: 25.4 },
    ]);
    expect(v).toHaveLength(60);
    expect(v[7]).toBeCloseTo(0.4, 5);
    expect(interpolateNowcast([])).toEqual(Array(60).fill(0));
  });
});
