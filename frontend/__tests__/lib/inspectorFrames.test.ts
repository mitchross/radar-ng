import { inspectSourceFor } from "../../src/lib/inspector";

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
  });
});
