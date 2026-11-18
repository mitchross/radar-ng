import { describe, expect, it } from "vitest";
import { bodyAltitude, twilight } from "./astro";
import {
  compassPoint,
  inSkyWindow,
  parseSkyTonight,
  planIssAlerts,
  planetsTonight,
  skyTonightCard,
  skyTonightPath,
  skyVerdict,
  tonight,
  type HourlyClouds,
} from "./skyTonight";

const DETROIT = { lat: 42.3314, lon: -83.0458 };
const TZ = "America/Detroit";
const OFFSET = -5 * 3600;
const clock = { timeZone: TZ, utcOffsetSeconds: OFFSET };

// Server body for Detroit; the passes are Heavens-Above's visible ones (UTC).
const BODY = {
  status: "ok",
  elements_epoch: "2026-11-17T12:40:23Z",
  source: "Orbital elements: CelesTrak",
  passes: [
    { start: "2026-11-17T23:47:42Z", max_at: "2026-11-17T23:50:20Z", end: "2026-11-17T23:50:53Z", max_elevation_deg: 21, start_azimuth_deg: 294, max_azimuth_deg: 345, end_azimuth_deg: 0 },
    { start: "2026-11-18T22:59:25Z", max_at: "2026-11-18T23:02:17Z", end: "2026-11-18T23:04:53Z", max_elevation_deg: 25.5, start_azimuth_deg: 282, max_azimuth_deg: 341, end_azimuth_deg: 37 },
    { start: "bad", max_at: "2026-11-18T23:02:17Z", end: "2026-11-18T23:04:53Z", max_elevation_deg: 25, start_azimuth_deg: 1, max_azimuth_deg: 1, end_azimuth_deg: 1 },
  ],
};

/** Hourly wall times at Detroit for 17-19 Nov with one cloud value for every hour. */
function hourly(cloud: (hourUtc: number) => number): HourlyClouds {
  const time: string[] = [];
  const cloud_cover: number[] = [];
  for (let h = 0; h < 72; h++) {
    const utc = Date.UTC(2026, 10, 17, 5 + h);
    const wall = new Date(utc + OFFSET * 1000).toISOString().slice(0, 16);
    time.push(wall);
    cloud_cover.push(cloud(utc));
  }
  return { time, cloud_cover };
}

const at = (iso: string) => Date.parse(iso);

describe("parseSkyTonight", () => {
  it("keeps well-formed passes in order and drops the rest", () => {
    const r = parseSkyTonight(BODY)!;
    expect(r.status).toBe("ok");
    expect(r.passes).toHaveLength(2);
    expect(r.passes[0].start).toBe(at("2026-11-17T23:47:42Z"));
  });
  it("stale or missing elements give no passes", () => {
    expect(parseSkyTonight({ ...BODY, status: "stale" })!.passes).toEqual([]);
    expect(parseSkyTonight({ status: "no_data", passes: [] })!.status).toBe("no_data");
    expect(parseSkyTonight(null)).toBeNull();
  });
});

describe("tonight", () => {
  it("is the night after today's sunset, or the one still running after midnight", () => {
    const evening = tonight(at("2026-11-17T20:00:00Z"), "2026-11-17", DETROIT.lat, DETROIT.lon)!;
    const early = tonight(at("2026-11-18T09:00:00Z"), "2026-11-18", DETROIT.lat, DETROIT.lon)!;
    expect(early).toEqual(evening);
    // USNO rstt/oneday, Detroit 18 Nov 2026: sunrise 7:27 AM EST, civil twilight starts 6:56 AM.
    expect(Math.abs(evening.darkEnd - at("2026-11-18T11:56:00Z"))).toBeLessThan(60_000);
    expect(Math.abs(evening.sunrise - at("2026-11-18T12:27:00Z"))).toBeLessThan(60_000);
    expect(evening.darkStart).toBeGreaterThan(evening.sunset);
  });
  it("shows the card from two hours before sunset until sunrise", () => {
    const n = tonight(at("2026-11-17T20:00:00Z"), "2026-11-17", DETROIT.lat, DETROIT.lon)!;
    expect(inSkyWindow(n.sunset - 2 * 3600_000 - 1, n)).toBe(false);
    expect(inSkyWindow(n.sunset - 2 * 3600_000, n)).toBe(true);
    expect(inSkyWindow(n.sunrise - 1, n)).toBe(true);
    expect(inSkyWindow(n.sunrise, n)).toBe(false);
  });
  it("is null in polar night", () => {
    expect(tonight(at("2026-12-21T12:00:00Z"), "2026-12-21", 78.2, 15.6)).toBeNull();
  });
});

describe("skyVerdict", () => {
  const n = tonight(at("2026-11-17T20:00:00Z"), "2026-11-17", DETROIT.lat, DETROIT.lon)!;
  it("averages the dark hours", () => {
    expect(skyVerdict(hourly(() => 10), n, n.sunset, OFFSET)).toBe("clear");
    expect(skyVerdict(hourly(() => 50), n, n.sunset, OFFSET)).toBe("partly");
    expect(skyVerdict(hourly(() => 95), n, n.sunset, OFFSET)).toBe("cloudy");
    expect(skyVerdict({ time: [] }, n, n.sunset, OFFSET)).toBeNull();
  });
});

describe("sky bodies", () => {
  it("the sun is 6° down when civil twilight ends", () => {
    const end = twilight("2026-11-17", DETROIT.lat, DETROIT.lon, "civil").set!;
    expect(bodyAltitude("Sun", DETROIT.lat, DETROIT.lon, end)).toBeCloseTo(-6, 0);
  });
  it("lists planets that really are 10° up", () => {
    const n = tonight(at("2026-11-17T20:00:00Z"), "2026-11-17", DETROIT.lat, DETROIT.lon)!;
    const list = planetsTonight(n, DETROIT.lat, DETROIT.lon);
    // Saturn is two months past opposition (Oct 2026): up in the south-east at dusk.
    expect(list.map((p) => p.name)).toContain("Saturn");
    for (const p of list) expect(bodyAltitude(p.name, DETROIT.lat, DETROIT.lon, p.from)).toBeGreaterThanOrEqual(10);
  });
  it("names compass points", () => {
    expect([0, 22.5, 294, 359].map(compassPoint)).toEqual(["N", "NNE", "WNW", "N"]);
  });
});

describe("skyTonightCard", () => {
  const now = at("2026-11-17T21:00:00Z");
  const night = tonight(now, "2026-11-17", DETROIT.lat, DETROIT.lon)!;
  const base = { nowMs: now, night, ...DETROIT, hourly: hourly(() => 5), clock };

  it("lists tonight's pass with local time and directions", () => {
    const card = skyTonightCard({ ...base, report: parseSkyTonight(BODY) });
    expect(card.verdictText).toBe("Clear, go look");
    expect(card.passes).toEqual(["6:47 PM · WNW to N · 21° high · 3 min"]);
    expect(card.issNote).toBeNull();
    // USNO: first quarter 17 Nov 6:48 AM; the moon sets at 12:32 AM on 18 Nov.
    expect(card.moon).toMatch(/^First quarter · 5\d% lit · sets 12:3[123] AM$/);
    expect(card.credit).toBe("Orbital elements: CelesTrak");
  });
  it("stale elements hide the ISS lines and say why", () => {
    const card = skyTonightCard({ ...base, report: parseSkyTonight({ ...BODY, status: "stale" }) });
    expect(card.passes).toEqual([]);
    expect(card.issNote).toMatch(/more than 14 days old/);
  });
  it("points at the next pass when tonight has none", () => {
    const later = at("2026-11-18T00:00:00Z");
    const card = skyTonightCard({ ...base, nowMs: later, report: parseSkyTonight(BODY) });
    expect(card.issNote).toMatch(/^No visible ISS pass tonight\. Next: Wed 5:59 PM\.$/);
  });
});

describe("planIssAlerts", () => {
  const r = parseSkyTonight(BODY);
  it("fires 10 minutes before good passes only", () => {
    const plans = planIssAlerts(r, hourly(() => 5), at("2026-11-17T21:00:00Z"), clock);
    expect(plans).toHaveLength(1);
    expect(plans[0].fireAt).toBe(at("2026-11-18T22:49:25Z"));
    expect(plans[0].body).toBe("Look WNW at 5:59 PM. It climbs to 26° and sets NE.");
  });
  it("skips cloudy passes and passes already started", () => {
    expect(planIssAlerts(r, hourly(() => 90), at("2026-11-17T21:00:00Z"), clock)).toEqual([]);
    expect(planIssAlerts(r, null, at("2026-11-18T22:55:00Z"), clock)).toEqual([]);
  });
  it("rounds coordinates in the path", () => {
    expect(skyTonightPath(42.33141, -83.04583)).toBe("/api/sky/tonight/42.33/-83.05");
  });
});
