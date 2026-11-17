import { stationLabel, whiteChristmasCard } from "../../src/lib/whiteChristmas";

describe("whiteChristmas (shared with web)", () => {
  const station = { id: "USW00014922", name: "MINNEAPOLIS/ST PAUL AP, MN US", lat: 44.8831, lon: -93.2289, pct: 73.6, years: 30, distance_mi: 9.6 };

  it("writes the same sentence as the web card", () => {
    expect(whiteChristmasCard("2026-12-10", station)?.sentence).toBe(
      "74% of years had snow on the ground on Dec 25 at Minneapolis/St Paul Airport, MN, 10 mi away",
    );
    expect(stationLabel(station.name)).toBe("Minneapolis/St Paul Airport, MN");
  });

  it("hides out of season", () => {
    expect(whiteChristmasCard("2026-12-26", station)).toBeNull();
  });
});
