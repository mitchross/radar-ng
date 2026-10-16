import { formatReading, inspectSourceFor } from "../../src/lib/inspector";

describe("inspectSourceFor", () => {
  it("reads radar-family frames from their own grids", () => {
    expect(inspectSourceFor("radar", "mrms")).toBe("radar");
    expect(inspectSourceFor("radar", undefined)).toBe("radar");
    expect(inspectSourceFor("radar", "nowcast")).toBe("nowcast");
    expect(inspectSourceFor("radar", "radar-hrrr")).toBe("radar-hrrr");
    expect(inspectSourceFor("radar-hrrr", "nowcast")).toBe("nowcast");
  });
  it("leaves other layers alone", () => {
    expect(inspectSourceFor("temperature", "radar-hrrr")).toBe("temperature");
    expect(inspectSourceFor("air-quality", null)).toBe("air-quality");
    expect(inspectSourceFor("precip-24h", "precip-24h")).toBe("precip-24h");
  });
});

describe("formatReading", () => {
  it("shows the past-24-hour total in inches and a dash when dry", () => {
    const reading = (value: number) => ({ ok: true, value, unit: "in", source: "grid" as const });
    expect(formatReading("precip-24h", reading(1.3405))).toBe("1.34 in");
    expect(formatReading("precip-24h", reading(0.004))).toBe("—");
  });
});
