import { parseSnowSpot, snowCardShows, snowSpotPath, snowSpotView } from "../../src/lib/snowSpot";

// The same contract fixture the Rust test writes and the web test reads.
const contract = require("../../../web/src/shared/testdata/snowSpot.contract.json");

const now = Date.parse(contract.now);
const clock = { timeZone: contract.time_zone, utcOffsetSeconds: contract.utc_offset_seconds };

describe("snow at your spot (app)", () => {
  it("builds the same card as the web from the server contract", () => {
    const spot = parseSnowSpot(contract.full)!;
    expect(snowCardShows(spot, now)).toBe(true);
    const v = snowSpotView(spot, now, clock);
    expect(v.headline).toBe("0.1–5 in by Friday 8 pm");
    expect(v.bars).toHaveLength(48);
    expect(v.lines).toContain("60% chance of 4 in or more");
  });

  it("all layers missing is a valid answer and no card error", () => {
    const spot = parseSnowSpot(contract.empty)!;
    expect(snowCardShows(spot, now)).toBe(false);
    expect(() => snowSpotView(spot, now, clock)).not.toThrow();
  });

  it("keeps the card when only the layers answered", () => {
    const spot = parseSnowSpot(contract.layers_only)!;
    expect(snowCardShows(spot, now)).toBe(true);
    expect(snowSpotView(spot, now, clock).bars).toEqual([]);
  });

  it("calls only our server's path", () => {
    expect(snowSpotPath(39.74, -105.99)).toBe("/api/snow/39.7/-106.0");
  });
});
