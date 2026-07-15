import { interpolateRadarNowcast } from "../../src/lib/radarNowcast";

describe("interpolateRadarNowcast", () => {
  it("returns an honest dry hour when no point samples are available", () => {
    expect(interpolateRadarNowcast([])).toEqual(Array(60).fill(0));
  });

  it("smoothly expands five-minute MRMS point samples", () => {
    const result = interpolateRadarNowcast([
      { timestamp: "T+5", lead_minutes: 5, dbz: 20, precipitation_mm_h: 25.4 },
      { timestamp: "T+10", lead_minutes: 10, dbz: 30, precipitation_mm_h: 76.2 },
    ], 11);

    expect(result).toHaveLength(11);
    expect(result[0]).toBe(1);
    expect(result[5]).toBe(1);
    expect(result[7]).toBeCloseTo(1.8);
    expect(result[10]).toBeCloseTo(3);
  });
});
