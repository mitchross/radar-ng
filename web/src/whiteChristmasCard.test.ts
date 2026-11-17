import { describe, expect, it } from "vitest";
import { whiteChristmasCardHtml } from "./whiteChristmasCard";

const station = { id: "USW00094847", name: "DETROIT METRO AP, MI US", lat: 42.23, lon: -83.33, pct: 34.7, years: 30, distance_mi: 18.4 };

describe("whiteChristmasCardHtml", () => {
  it("shows the odds, the station, the credit and the map link", () => {
    const html = whiteChristmasCardHtml("2026-12-20", station, false);
    expect(html).toContain("35% of years had snow on the ground on Dec 25 at Detroit Metro Airport, MI, 18 mi away");
    expect(html).toContain("NOAA NCEI");
    expect(html).toContain("See stations on map");
    expect(whiteChristmasCardHtml("2026-12-20", station, true)).toContain("Hide stations on map");
  });

  it("renders nothing out of season or when the station is too far", () => {
    expect(whiteChristmasCardHtml("2026-10-01", station, false)).toBe("");
    expect(whiteChristmasCardHtml("2026-12-20", { ...station, distance_mi: 80 }, false)).toBe("");
  });
});
