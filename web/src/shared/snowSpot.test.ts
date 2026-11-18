import { describe, expect, it } from "vitest";
import contract from "./testdata/snowSpot.contract.json";
import { ByteCappedCache, parseSnowSpot, snowCardShows, snowLikely, snowRange, snowSpotPath, snowSpotView, snowWhen } from "./snowSpot";

const now = Date.parse(contract.now);
const clock = { timeZone: contract.time_zone, utcOffsetSeconds: contract.utc_offset_seconds };
const view = (k: "full" | "hourly_only" | "layers_only" | "empty") => snowSpotView(parseSnowSpot(contract[k])!, now, clock);

describe("snow at your spot (server contract)", () => {
  it("reads every field of a full answer", () => {
    const spot = parseSnowSpot(contract.full)!;
    expect(spot.hourly?.time).toHaveLength(72);
    expect(spot.onGroundIn).toBe(6.2);
    expect(spot.nbm72hIn).toBe(5.25);
    expect(spot.chance).toEqual([{ inches: 4, pct: 60, windowEnd: Date.parse("2026-11-19T06:00:00Z") }]);
    expect(snowCardShows(spot, now)).toBe(true);
    const v = view("full");
    expect(v.headline).toBe("0.1–5 in by Friday 8 pm");
    expect(v.window).toBe("Snowing now · ends Friday 8 pm");
    expect(v.bars).toHaveLength(48);
    expect(v.lines).toEqual(["6.2 in on the ground", "1.5 in fell in the last 24 h", "Ice up to 0.12 in", "60% chance of 4 in or more"]);
    expect(v.credit).toBe("Open-Meteo · NOAA SNODAS · NOAA NOHRSC · NOAA NBM · NOAA WPC");
  });

  it("drops lines, not the card, when layers are missing", () => {
    const v = view("hourly_only");
    expect(v.lines).toEqual([]);
    expect(v.bars).toHaveLength(48);
    expect(v.credit).toBe("Open-Meteo");
    const l = view("layers_only");
    expect(l.headline).toBe("5 in by Friday 9 pm");
    expect(l.window).toBeUndefined();
    expect(l.bars).toEqual([]);
  });

  it("all layers missing is still a valid answer with no card error", () => {
    const spot = parseSnowSpot(contract.empty)!;
    expect(spot).toEqual({ chance: [], sources: [] } as unknown as typeof spot);
    expect(snowCardShows(spot, now)).toBe(false);
    expect(() => view("empty")).not.toThrow();
    expect(view("empty")).toMatchObject({ headline: "No snow expected", bars: [], lines: [], credit: "" });
    expect(parseSnowSpot(null)).toBeNull();
    const bad = parseSnowSpot({ hourly: { time: ["x"], snowfall_in: [1] }, chance: [{ pct: "a" }] })!;
    expect(bad.hourly).toBeUndefined();
    expect(bad.chance).toEqual([]);
  });
});

describe("snow card words", () => {
  it("shows one number when the totals agree within half an inch", () => {
    expect(snowRange(3.2, 3.6)).toBe("3 in");
    expect(snowRange(3, 5)).toBe("3–5 in");
    expect(snowRange(undefined, 0.4)).toBe("0.4 in");
    expect(snowRange(0.02, undefined)).toBeUndefined();
  });

  it("names the day unless it is today", () => {
    const c = { timeZone: "America/Detroit", utcOffsetSeconds: -18000 };
    const t = Date.parse("2026-11-19T12:00:00Z");
    expect(snowWhen(t, t - 3_600_000, c)).toBe("7 am");
    expect(snowWhen(t, t - 86_400_000, c)).toBe("Thursday 7 am");
    expect(snowWhen(t, t - 86_400_000, { utcOffsetSeconds: -18000 })).toBe("Thursday 7 am");
  });

  it("reserves space only when the forecast has snow codes", () => {
    const hourly = { time: ["2026-11-18T00:00", "2026-11-18T01:00"], weather_code: [3, 73] };
    expect(snowLikely(hourly, Date.parse("2026-11-18T05:00:00Z"), -18000)).toBe(true);
    expect(snowLikely({ ...hourly, weather_code: [3, 61] }, Date.parse("2026-11-18T05:00:00Z"), -18000)).toBe(false);
  });

  it("asks for the server's 0.1 degree cell", () => {
    expect(snowSpotPath(42.9634, -85.6681)).toBe("/api/snow/43.0/-85.7");
  });
});

describe("ByteCappedCache", () => {
  it("evicts the oldest entries past the byte cap and expires by age", () => {
    const c = new ByteCappedCache<string>(10, 1000);
    c.set("a", "a", 4, 0);
    c.set("b", "b", 4, 0);
    c.get("a", 0);
    c.set("c", "c", 4, 0);
    expect(c.get("b", 0)).toBeUndefined();
    expect(c.get("a", 0)).toBe("a");
    expect(c.bytes).toBe(8);
    expect(c.get("a", 2000)).toBeUndefined();
    c.set("huge", "x", 11, 0);
    expect(c.get("huge", 0)).toBeUndefined();
  });
});
