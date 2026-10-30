import { describe, expect, it } from "vitest";
import { forecastTimeMs, sunTimesMs, wallClock } from "./shared/forecastTime";
import { forecastHourLabel } from "./weather";

describe("forecast times use the location's zone", () => {
  it("restores the instant from wall time plus offset", () => {
    expect(forecastTimeMs("2026-10-11T14:00", -14400)).toBe(Date.UTC(2026, 9, 11, 18, 0));
    expect(forecastTimeMs("2026-10-11", 3600)).toBe(Date.UTC(2026, 9, 10, 23, 0));
    expect(forecastTimeMs("nonsense", -14400)).toBeNaN();
  });

  it("reads an instant as wall time at the location", () => {
    expect(wallClock(Date.UTC(2026, 9, 11, 3, 30), -14400)).toEqual({ hour: 23, dateKey: "2026-10-10" });
  });

  it("finds the sun times of the location's own day", () => {
    const daily = { time: ["2026-10-10", "2026-10-11"], sunrise: ["2026-10-10T07:00", null], sunset: ["2026-10-10T18:30", null] };
    const at = Date.UTC(2026, 9, 11, 3, 30); // 23:30 on the 10th at -04:00
    expect(sunTimesMs(daily, at, -14400)).toEqual({ sunrise: Date.UTC(2026, 9, 10, 11, 0), sunset: Date.UTC(2026, 9, 10, 22, 30) });
    expect(sunTimesMs(daily, at + 86_400_000, -14400).sunrise).toBeNaN();
  });

  it("labels the hour at the location, not the browser", () => {
    expect(forecastHourLabel("2026-10-11T14:00", -14400)).toBe("2pm");
    expect(forecastHourLabel("2026-10-11T00:00", 3600)).toBe("12am");
  });
});
