import { describe, expect, it } from "vitest";
import {
  APPROACH_SEEN_TTL_MS,
  approachCopy,
  distanceKm,
  stormApproaches,
  stormCone,
  stormConeFeatures,
  stormLayerOpacity,
  stormTrackTimeSec,
  takeNewApproaches,
  type StormCell,
} from "./stormCone";

const cell = (over: Partial<StormCell> = {}): StormCell => ({
  id: 1, lon: -86.2, lat: 42.96, peakDbz: 58, areaKm2: 120, speedMph: 37.3, bearing: 90, confidence: 0.9,
  eastKmh: 60, northKmh: 0, history: [[0, -86.4, 42.96], [300, -86.3, 42.96], [600, -86.2, 42.96]], ...over,
});
const place: [number, number] = [-85.67, 42.96];

describe("stormCone", () => {
  it("is a closed wedge from the cell, +-15 deg, 60 km long at 60 km/h", () => {
    const ring = stormCone(cell())!;
    expect(ring[0]).toEqual([-86.2, 42.96]);
    expect(ring[ring.length - 1]).toEqual(ring[0]);
    expect(ring).toHaveLength(9);
    const reach = ring.slice(1, -1).map((p) => distanceKm([-86.2, 42.96], p));
    for (const r of reach) expect(r).toBeCloseTo(60, 0);
    // Middle vertex is the 60-minute tick of the track, due east.
    expect(ring[4][1]).toBeCloseTo(42.96, 6);
    // Edges sit 60 * sin(15deg) ~ 15.5 km off the centre line.
    expect(distanceKm(ring[1], ring[7])).toBeCloseTo(2 * 60 * Math.sin((15 * Math.PI) / 180), 0);
  });

  it("is deterministic: same input, same polygon (golden, shared by web and app)", () => {
    expect(JSON.stringify(stormCone(cell()))).toBe(JSON.stringify(stormCone(cell())));
    expect(stormCone(cell())!.map(([lon, lat]) => [lon.toFixed(4), lat.toFixed(4)])).toEqual(GOLDEN);
  });

  it("only for tracked cells", () => {
    expect(stormConeFeatures([cell(), cell({ id: 2, confidence: 0.1 }), cell({ id: 3, speedMph: 1 }), cell({ id: 4, history: [] }), cell({ id: 5, speedMph: 95 })])).toHaveLength(1);
  });
});

describe("stormLayerOpacity", () => {
  it("fades past 10 minutes and returns at the live frame", () => {
    const t = 1_000_000;
    expect(stormLayerOpacity(t, t)).toBe(1);
    expect(stormLayerOpacity(t - 600, t)).toBe(1);
    expect(stormLayerOpacity(t - 601, t)).toBeLessThan(1);
    expect(stormLayerOpacity(t + 3600, t)).toBeLessThan(1);
    expect(stormLayerOpacity(null, t)).toBe(1);
    expect(stormLayerOpacity(t, null)).toBe(1);
  });
  it("reads the track time from the document", () => {
    expect(stormTrackTimeSec({ timestamp: "2026-11-06T12:00:00Z" })).toBe(Date.parse("2026-11-06T12:00:00Z") / 1000);
    expect(stormTrackTimeSec({})).toBeNull();
  });
});

describe("stormApproaches", () => {
  it("approaching: a 55+ dBZ tracked cell heading for you", () => {
    // ~43 km west at 60 km/h -> within 10 km after ~33 min is too late; use a closer cell.
    const [a] = stormApproaches([cell({ lon: -85.95 })], place);
    expect(a.id).toBe(1);
    expect(a.minutes).toBeGreaterThan(10);
    expect(a.minutes).toBeLessThanOrEqual(30);
  });
  it("too far to reach within 30 minutes", () => {
    expect(stormApproaches([cell()], place)).toEqual([]);
  });
  it("passing wide: path stays more than 10 km away", () => {
    expect(stormApproaches([cell({ lon: -85.95, lat: 43.2 })], place)).toEqual([]);
  });
  it("receding: moving away", () => {
    expect(stormApproaches([cell({ lon: -85.3, eastKmh: 60 })], place)).toEqual([]);
    expect(stormApproaches([cell({ lon: -85.95, eastKmh: -60, bearing: 270 })], place)).toEqual([]);
  });
  it("needs 55 dBZ and the tracked rule", () => {
    expect(stormApproaches([cell({ lon: -85.95, peakDbz: 54 })], place)).toEqual([]);
    expect(stormApproaches([cell({ lon: -85.95, confidence: 0.2 })], place)).toEqual([]);
  });
  it("already overhead is minute 0", () => {
    expect(stormApproaches([cell({ lon: -85.68 })], place)[0].minutes).toBe(0);
  });
});

describe("takeNewApproaches", () => {
  const a = { id: 9, minutes: 20, peakDbz: 60, speedMph: 30, bearing: 90 };
  it("announces a cell once, not per poll", () => {
    const first = takeNewApproaches([a], {}, 1000);
    expect(first.fresh).toEqual([a]);
    const again = takeNewApproaches([a], first.seen, 61_000);
    expect(again.fresh).toEqual([]);
    expect(takeNewApproaches([{ ...a, id: 10 }], again.seen, 62_000).fresh).toHaveLength(1);
  });
  it("forgets a cell id after the TTL", () => {
    const first = takeNewApproaches([a], {}, 0);
    expect(takeNewApproaches([a], first.seen, APPROACH_SEEN_TTL_MS + 1).fresh).toEqual([a]);
  });
});

describe("approachCopy", () => {
  it("says estimate, not NWS, and follow NWS", () => {
    const { title, body } = approachCopy({ id: 1, minutes: 20, peakDbz: 61, speedMph: 30, bearing: 90 });
    expect(title).toContain("~20 min");
    expect(body).toMatch(/radar estimate, not an NWS warning/i);
    expect(body).toMatch(/follow NWS warnings/i);
  });
});

const GOLDEN: string[][] = [["-86.2000","42.9600"],["-85.4886","43.1004"],["-85.4747","43.0542"],["-85.4663","43.0073"],["-85.4635","42.9600"],["-85.4663","42.9127"],["-85.4747","42.8658"],["-85.4886","42.8196"],["-86.2000","42.9600"]];
