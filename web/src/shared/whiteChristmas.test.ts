import { describe, expect, it } from "vitest";
import {
  inWhiteChristmasSeason,
  parseNearestStation,
  stationLabel,
  whiteChristmasCard,
  whiteChristmasPath,
  type WhiteChristmasStation,
} from "./whiteChristmas";

const grr: WhiteChristmasStation = { id: "USW00094860", name: "GRAND RAPIDS, MI US", lat: 42.8939, lon: -85.5447, pct: 53.4, years: 30, distance_mi: 6.2 };

describe("whiteChristmas", () => {
  it("is in season from Nov 15 to Dec 25 only", () => {
    expect(inWhiteChristmasSeason("2026-11-14")).toBe(false);
    expect(inWhiteChristmasSeason("2026-11-15")).toBe(true);
    expect(inWhiteChristmasSeason("2026-12-25")).toBe(true);
    expect(inWhiteChristmasSeason("2026-12-26")).toBe(false);
    expect(inWhiteChristmasSeason("2027-07-04")).toBe(false);
  });

  it("writes the card sentence", () => {
    expect(whiteChristmasCard("2026-12-01", grr)?.sentence).toBe("53% of years had snow on the ground on Dec 25 at Grand Rapids, MI, 6 mi away");
    expect(whiteChristmasCard("2026-12-01", { ...grr, distance_mi: 0.3 })?.detail).toBe("at Grand Rapids, MI, under 1 mi away");
  });

  it("hides outside the season, with no station, or past 60 miles", () => {
    expect(whiteChristmasCard("2026-12-26", grr)).toBeNull();
    expect(whiteChristmasCard("2026-12-01", null)).toBeNull();
    expect(whiteChristmasCard("2026-12-01", { ...grr, distance_mi: 60 })).not.toBeNull();
    expect(whiteChristmasCard("2026-12-01", { ...grr, distance_mi: 60.1 })).toBeNull();
  });

  it("tidies NCEI station names", () => {
    expect(stationLabel("DETROIT METRO AP, MI US")).toBe("Detroit Metro Airport, MI");
    expect(stationLabel("MINNEAPOLIS/ST PAUL AP, MN US")).toBe("Minneapolis/St Paul Airport, MN");
    expect(stationLabel("NEW YORK CNTRL PK TWR, NY US")).toBe("New York Central Park Tower, NY");
    expect(stationLabel("DENVER-STAPLETON, CO US")).toBe("Denver-Stapleton, CO");
  });

  it("parses the server's nearest-station body and rejects junk", () => {
    expect(parseNearestStation({ station: grr })).toEqual(grr);
    expect(parseNearestStation({ station: null })).toBeNull();
    expect(parseNearestStation({ station: { ...grr, pct: "53" } })).toBeNull();
    expect(parseNearestStation(null)).toBeNull();
  });

  it("rounds the coordinates it sends", () => {
    expect(whiteChristmasPath(42.963412, -85.668123)).toBe("/api/climate/white-christmas?lat=42.96&lon=-85.67");
  });
});
