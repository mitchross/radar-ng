import { describe, expect, it } from "vitest";
import { countdownRows, daysBetween, holidaysForYear, upcomingHolidays, zoneObservesDst } from "./holidays";

// Published calendars 2026–2030 (federal holidays per 5 U.S.C. 6103, Easter, US clock changes).
const PINNED: Record<string, string[]> = {
  "new-year": ["2026-01-01", "2027-01-01", "2028-01-01", "2029-01-01", "2030-01-01"],
  mlk: ["2026-01-19", "2027-01-18", "2028-01-17", "2029-01-15", "2030-01-21"],
  presidents: ["2026-02-16", "2027-02-15", "2028-02-21", "2029-02-19", "2030-02-18"],
  "dst-start": ["2026-03-08", "2027-03-14", "2028-03-12", "2029-03-11", "2030-03-10"],
  easter: ["2026-04-05", "2027-03-28", "2028-04-16", "2029-04-01", "2030-04-21"],
  memorial: ["2026-05-25", "2027-05-31", "2028-05-29", "2029-05-28", "2030-05-27"],
  juneteenth: ["2026-06-19", "2027-06-19", "2028-06-19", "2029-06-19", "2030-06-19"],
  independence: ["2026-07-04", "2027-07-04", "2028-07-04", "2029-07-04", "2030-07-04"],
  labor: ["2026-09-07", "2027-09-06", "2028-09-04", "2029-09-03", "2030-09-02"],
  columbus: ["2026-10-12", "2027-10-11", "2028-10-09", "2029-10-08", "2030-10-14"],
  halloween: ["2026-10-31", "2027-10-31", "2028-10-31", "2029-10-31", "2030-10-31"],
  "dst-end": ["2026-11-01", "2027-11-07", "2028-11-05", "2029-11-04", "2030-11-03"],
  veterans: ["2026-11-11", "2027-11-11", "2028-11-11", "2029-11-11", "2030-11-11"],
  thanksgiving: ["2026-11-26", "2027-11-25", "2028-11-23", "2029-11-22", "2030-11-28"],
  christmas: ["2026-12-25", "2027-12-25", "2028-12-25", "2029-12-25", "2030-12-25"],
};

describe("holidaysForYear", () => {
  for (const year of [2026, 2027, 2028, 2029, 2030]) {
    it(`pins every date in ${year}`, () => {
      const got = Object.fromEntries(holidaysForYear(year).map((h) => [h.id, h.dateKey]));
      const want = Object.fromEntries(Object.entries(PINNED).map(([id, dates]) => [id, dates[year - 2026]]));
      expect(got).toEqual(want);
    });
  }

  it("lists in date order", () => {
    const keys = holidaysForYear(2027).map((h) => h.dateKey);
    expect(keys).toEqual([...keys].sort());
  });
});

describe("upcomingHolidays", () => {
  it("counts today and rolls into next year", () => {
    expect(upcomingHolidays("2026-12-25", 3).map((h) => h.id)).toEqual(["christmas", "new-year", "mlk"]);
  });

  it("can skip the clock changes", () => {
    expect(upcomingHolidays("2026-10-30", 2, { clockChanges: false }).map((h) => h.id)).toEqual(["halloween", "veterans"]);
    expect(upcomingHolidays("2026-10-30", 2).map((h) => h.id)).toEqual(["halloween", "dst-end"]);
  });
});

describe("daysBetween", () => {
  it("counts calendar days across a clock change and a leap day", () => {
    expect(daysBetween("2026-11-17", "2026-11-26")).toBe(9);
    expect(daysBetween("2028-02-28", "2028-03-01")).toBe(2);
    expect(daysBetween("2026-03-07", "2026-03-09")).toBe(2);
  });
});

describe("zoneObservesDst", () => {
  it("knows Arizona and Hawaii keep one offset", () => {
    expect(zoneObservesDst("America/Detroit")).toBe(true);
    expect(zoneObservesDst("America/Phoenix")).toBe(false);
    expect(zoneObservesDst("Pacific/Honolulu")).toBe(false);
    expect(zoneObservesDst(undefined)).toBe(true);
    expect(zoneObservesDst("Not/AZone")).toBe(true);
  });
});

describe("countdownRows", () => {
  const detroit = { lat: 42.3314, lon: -83.0458, timeZone: "America/Detroit", utcOffsetSeconds: -18000 };

  it("labels the next holidays with the sunset in the location's zone", () => {
    const rows = countdownRows("2026-11-17", detroit, 3);
    expect(rows.map((r) => [r.name, r.dateLabel, r.inLabel])).toEqual([
      ["Thanksgiving", "Thu, Nov 26", "in 9 days"],
      ["Christmas", "Fri, Dec 25", "in 38 days"],
      ["New Year's Day", "Fri, Jan 1", "in 45 days"],
    ]);
    // USNO: Detroit sunset on 2026-12-25 is 17:05 EST.
    expect(rows[1].sunsetLabel).toBe("Sunset 5:05 PM");
  });

  it("says Today and Tomorrow", () => {
    expect(countdownRows("2026-12-25", detroit, 1)[0].inLabel).toBe("Today");
    expect(countdownRows("2026-12-24", detroit, 1)[0].inLabel).toBe("Tomorrow");
  });

  it("drops the clock change in Phoenix", () => {
    const phoenix = { lat: 33.4484, lon: -112.074, timeZone: "America/Phoenix" };
    expect(countdownRows("2026-10-30", phoenix, 2).map((r) => r.id)).toEqual(["halloween", "veterans"]);
  });

  it("uses the fixed offset when no zone name came back", () => {
    const rows = countdownRows("2026-12-20", { lat: 42.3314, lon: -83.0458, utcOffsetSeconds: -18000 }, 1);
    expect(rows[0].sunsetLabel).toBe("Sunset 5:05 PM");
  });
});
