import { describe, expect, it } from "vitest";
import { interpolateNowcast, nowcastVerdict } from "./nowcast";

describe("next-hour nowcast", () => {
  it("interpolates 5-minute leads to minutes in inches/hour", () => {
    const v = interpolateNowcast([{ lead_minutes: 5, precipitation_mm_h: 0 }, { lead_minutes: 10, precipitation_mm_h: 25.4 }]);
    expect(v).toHaveLength(60);
    expect(v[10]).toBeCloseTo(1, 5);
    expect(v[7]).toBeCloseTo(0.4, 5);
  });

  it("uses the app's verdict wording", () => {
    expect(nowcastVerdict(Array(60).fill(0))).toBe("No rain expected for the next hour.");
    const starting = Array(60).fill(0).map((_, i) => (i >= 20 && i < 40 ? (i === 30 ? 0.5 : 0.2) : 0));
    expect(nowcastVerdict(starting)).toBe("Rain starts in 20 minutes, peaks at 30 minutes, and ends near 39 minutes.");
  });
});
