import { advance, compass, describeIncoming, distanceKm, incomingStorm, parseStormCells, stormTrackGeoJSON, type StormCellInfo } from "../../src/lib/stormTracks";

const cell = (over: Partial<StormCellInfo> = {}): StormCellInfo => ({
  id: 1, lon: -86.2, lat: 42.96, peakDbz: 56, areaKm2: 120, speedMph: 37.3, bearing: 90, confidence: 0.9,
  eastKmh: 60, northKmh: 0, history: [[0, -86.4, 42.96], [300, -86.3, 42.96], [600, -86.2, 42.96]], ...over,
});

describe("stormTracks", () => {
  it("parses the /api/storms payload", () => {
    const [c] = parseStormCells({ features: [{ geometry: { type: "Point", coordinates: [-100, 45] }, properties: { cell_id: 7, peak_dbz: 58, tracking_confidence: 0.8, tracking_vector: { east_kmh: 30, north_kmh: 10, speed_kmh: 31.6, bearing_deg: 71.6 }, track_history: [[1, -100.1, 45]] } }] });
    expect(c).toMatchObject({ id: 7, peakDbz: 58, eastKmh: 30, bearing: 71.6 });
    expect(c.speedMph).toBeCloseTo(19.6, 1);
  });

  it("advances 60 km/h east by ~60 km in an hour", () => {
    const p = advance(-86, 43, 60, 0, 60);
    expect(distanceKm([-86, 43], p)).toBeCloseTo(60, 0);
  });

  it("draws paths only for confidently tracked, moving, history-backed cells", () => {
    const g = stormTrackGeoJSON([cell(), cell({ id: 2, confidence: 0.1 }), cell({ id: 3, speedMph: 1 }), cell({ id: 4, history: [] })]);
    expect(g.tracks.features).toHaveLength(1);
    expect(g.ticks.features.map((f) => f.properties?.label)).toEqual(["15", "30", "45", "60"]);
  });

  it("finds an incoming storm and when it arrives", () => {
    const t = incomingStorm([cell()], [-85.67, 42.96]);
    expect(t!.minutes).toBeGreaterThanOrEqual(30);
    expect(t!.minutes).toBeLessThanOrEqual(50);
    expect(incomingStorm([cell({ lon: -85.68 })], [-85.67, 42.96])?.minutes).toBe(0);
    expect(incomingStorm([cell({ eastKmh: -60 })], [-85.67, 42.96])).toBeNull();
    expect(describeIncoming({ minutes: 20, peakDbz: 61, speedMph: 30, bearing: 90 })).toEqual({ headline: "Hail-producing storm arriving in ~20 min", sub: "61 dBZ · moving E at 30 mph", severe: true });
    expect([0, 44, 91, 180, 270, 359].map(compass)).toEqual(["N", "NE", "E", "S", "W", "N"]);
  });
});
