import { describe, expect, it } from "vitest";
import { advance, compass, distanceKm, incomingStorm, lightningWithAge, parseStorms, stormGeoJSON, type StormCell } from "./overlays";

const cell = (over: Partial<StormCell> = {}): StormCell => ({
  history: [[0, -86.4, 42.96], [300, -86.3, 42.96], [600, -86.2, 42.96]],
  id: 1,
  lon: -86.2,
  lat: 42.96,
  peakDbz: 56,
  areaKm2: 120,
  speedMph: 37.3,
  bearing: 90,
  confidence: 0.9,
  eastKmh: 60,
  northKmh: 0,

  ...over,
});

describe("storm tracks", () => {
  it("parses the /api/storms payload", () => {
    const [c] = parseStorms({
      type: "FeatureCollection",
      features: [{
        type: "Feature",
        geometry: { type: "Point", coordinates: [-100, 45] },
        properties: { cell_id: 7, peak_dbz: 58, area_km2: 40, tracking_confidence: 0.8, tracking_vector: { east_kmh: 30, north_kmh: 10, speed_kmh: 31.6, bearing_deg: 71.6 }, track_history: [[1, -100.1, 45], [2, -100, 45]] },
      }],
    });
    expect(c).toMatchObject({ id: 7, peakDbz: 58, eastKmh: 30, bearing: 71.6 });
    expect(c.speedMph).toBeCloseTo(19.6, 1);
    expect(c.history).toHaveLength(2);
  });

  it("advances 60 km/h east by ~60 km in an hour", () => {
    const [lon, lat] = advance(-86, 43, 60, 0, 60);
    expect(distanceKm([-86, 43], [lon, lat])).toBeCloseTo(60, 0);
    expect(lat).toBeCloseTo(43, 6);
  });

  it("draws paths only for confidently tracked, moving cells", () => {
    const g = stormGeoJSON([cell(), cell({ id: 2, confidence: 0.1 }), cell({ id: 3, speedMph: 1 }), cell({ id: 4, history: [] })]);
    expect(g.cells.features).toHaveLength(4);
    expect(g.tracks.features).toHaveLength(1);
    expect(g.ticks.features.map((f) => f.properties?.label)).toEqual(["15", "30", "45", "60"]);
  });
});

describe("incomingStorm", () => {
  const grandRapids: [number, number] = [-85.67, 42.96];

  it("finds a cell whose path crosses the place and when", () => {
    // ~43 km west moving east at 60 km/h → ~40-45 min out.
    const t = incomingStorm([cell()], grandRapids);
    expect(t).not.toBeNull();
    expect(t!.minutes).toBeGreaterThanOrEqual(30);
    expect(t!.minutes).toBeLessThanOrEqual(50);
  });

  it("reports a cell already overhead as now", () => {
    expect(incomingStorm([cell({ lon: -85.68 })], grandRapids)?.minutes).toBe(0);
  });

  it("ignores cells moving away or untracked", () => {
    expect(incomingStorm([cell({ eastKmh: -60, bearing: 270 })], grandRapids)).toBeNull();
    expect(incomingStorm([cell({ confidence: 0 })], grandRapids)).toBeNull();
  });
});

describe("helpers", () => {
  it("labels compass bearings", () => {
    expect([0, 44, 91, 180, 270, 359].map(compass)).toEqual(["N", "NE", "E", "S", "W", "N"]);
  });

  it("adds strike age", () => {
    const fc = lightningWithAge({ type: "FeatureCollection", features: [{ type: "Feature", geometry: { type: "Point", coordinates: [0, 0] }, properties: { time: 100 } }] }, 160);
    expect(fc.features[0].properties?.age_s).toBe(60);
  });
});
