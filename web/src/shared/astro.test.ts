import { describe, expect, it } from "vitest";
import { bodyRiseSet, clockLabel, moonPhase, solarDayStartMs, sunTimes, twilight } from "./astro";

const MIN = 60_000;

// Published values: US Naval Observatory, aa.usno.navy.mil/api/rstt/oneday (minute precision).
const SUNSETS = [
  { place: "Detroit", date: "2026-12-25", lat: 42.3314, lon: -83.0458, tz: "America/Detroit", sunset: "5:05 PM", sunrise: "8:00 AM", civilEnd: "5:37 PM" },
  { place: "New York", date: "2027-07-04", lat: 40.7128, lon: -74.006, tz: "America/New_York", sunset: "8:31 PM", sunrise: "5:30 AM", civilEnd: "9:04 PM" },
  { place: "Phoenix", date: "2028-03-21", lat: 33.4484, lon: -112.074, tz: "America/Phoenix", sunset: "6:41 PM", sunrise: "6:30 AM", civilEnd: "7:06 PM" },
];

// Within a minute either side of the published, minute-rounded time.
const near = (atMs: number | null, label: string, tz: string) =>
  atMs != null && [atMs - MIN, atMs, atMs + MIN].some((t) => clockLabel(t, tz) === label);

describe("sunTimes", () => {
  for (const s of SUNSETS) {
    it(`matches USNO for ${s.place} on ${s.date}`, () => {
      const t = sunTimes(s.date, s.lat, s.lon);
      expect(near(t.set, s.sunset, s.tz)).toBe(true);
      expect(near(t.rise, s.sunrise, s.tz)).toBe(true);
      expect(near(twilight(s.date, s.lat, s.lon, "civil").set, s.civilEnd, s.tz)).toBe(true);
    });
  }

  it("has no sunset in polar night", () => {
    expect(sunTimes("2026-12-21", 78.22, 15.65)).toEqual({ rise: null, set: null });
  });

  it("rejects a bad date key", () => {
    expect(Number.isNaN(solarDayStartMs("2026-12", 0))).toBe(true);
    expect(sunTimes("nope", 42, -83)).toEqual({ rise: null, set: null });
  });
});

describe("twilight", () => {
  it("orders civil, nautical and astronomical dusk", () => {
    const [c, n, a] = (["civil", "nautical", "astronomical"] as const).map((k) => twilight("2026-11-17", 42.33, -83.05, k).set!);
    expect(c).toBeLessThan(n);
    expect(n).toBeLessThan(a);
  });
});

describe("moonPhase", () => {
  it("names a full and a new moon", () => {
    // Full moon 2026-12-24 01:28 UTC, new moon 2026-12-09 00:52 UTC (USNO phases).
    expect(moonPhase(Date.UTC(2026, 11, 24, 1, 28)).name).toBe("Full moon");
    expect(moonPhase(Date.UTC(2026, 11, 24, 1, 28)).illuminated).toBeGreaterThan(0.99);
    expect(moonPhase(Date.UTC(2026, 11, 9, 0, 52)).name).toBe("New moon");
  });
});

describe("bodyRiseSet", () => {
  it("finds a rise and a set within a day for each planet", () => {
    const from = Date.UTC(2026, 10, 17, 12);
    for (const body of ["Mercury", "Venus", "Mars", "Jupiter", "Saturn", "Moon"] as const) {
      const t = bodyRiseSet(body, 42.33, -83.05, from);
      expect(t.rise! - from).toBeGreaterThan(0);
      expect(t.rise! - from).toBeLessThan(86_400_000);
      expect(t.set! - from).toBeGreaterThan(0);
    }
  });
});

describe("clockLabel", () => {
  it("formats in the zone, with an offset fallback", () => {
    const t = Date.UTC(2026, 11, 25, 22, 5);
    expect(clockLabel(t, "America/Detroit")).toBe("5:05 PM");
    expect(clockLabel(t, undefined, -18000)).toBe("5:05 PM");
    expect(clockLabel(t, "Bad/Zone", -18000)).toBe("5:05 PM");
    expect(clockLabel(Date.UTC(2026, 11, 25, 5, 0), undefined, -18000)).toBe("12:00 AM");
  });
});
