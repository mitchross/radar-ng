import { describe, expect, it } from "vitest";
import { forecastHourLabel, forecastTimeMs } from "./weather";

describe("forecast times use the location's zone", () => {
  it("restores the instant from wall time plus offset", () => {
    expect(forecastTimeMs("2026-10-11T14:00", -14400)).toBe(Date.UTC(2026, 9, 11, 18, 0));
  });

  it("labels the hour at the location, not the browser", () => {
    expect(forecastHourLabel("2026-10-11T14:00", -14400)).toBe("2pm");
    expect(forecastHourLabel("2026-10-11T00:00", 3600)).toBe("12am");
  });
});
