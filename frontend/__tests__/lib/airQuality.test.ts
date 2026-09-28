import { aqiCategory, nearestFrameTimestamp, usAqiFromPm25 } from "../../src/lib/airQuality";

describe("usAqiFromPm25", () => {
  it("maps EPA 2024 breakpoints", () => {
    expect(usAqiFromPm25(0)).toBe(0);
    expect(usAqiFromPm25(6.9)).toBe(38);
    expect(usAqiFromPm25(9.0)).toBe(50);
    expect(usAqiFromPm25(9.1)).toBe(51);
    expect(usAqiFromPm25(35.4)).toBe(100);
    expect(usAqiFromPm25(55.5)).toBe(151);
    expect(usAqiFromPm25(400)).toBe(500);
  });

  it("labels categories", () => {
    expect(aqiCategory(38).label).toBe("Good");
    expect(aqiCategory(75).label).toBe("Moderate");
    expect(aqiCategory(120).label).toBe("Unhealthy for sensitive groups");
  });
});

describe("nearestFrameTimestamp", () => {
  const frames = [
    { timestamp: "2026-09-27T16:00:00+00:00" },
    { timestamp: "2026-09-27T17:00:00+00:00" },
    { timestamp: "2026-09-27T18:00:00+00:00" },
  ];
  it("picks the frame nearest to now", () => {
    expect(nearestFrameTimestamp(frames, Date.parse("2026-09-27T17:20:00Z"))).toBe("2026-09-27T17:00:00+00:00");
    expect(nearestFrameTimestamp(frames, Date.parse("2026-09-27T17:40:00Z"))).toBe("2026-09-27T18:00:00+00:00");
  });
  it("returns null when the run is stale or missing", () => {
    expect(nearestFrameTimestamp(frames, Date.parse("2026-09-29T12:00:00Z"))).toBeNull();
    expect(nearestFrameTimestamp(undefined, Date.now())).toBeNull();
  });
});
