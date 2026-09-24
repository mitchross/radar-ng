import {
  locationKey,
  PRECISION,
  roundCoord,
  roundCoords,
  roundNullableCoords,
} from "../../src/lib/coordinates";

describe("roundCoord", () => {
  it("truncates to the requested number of decimals", () => {
    expect(roundCoord(42.9634567, 2)).toBe(42.96);
    expect(roundCoord(42.9634567, 3)).toBe(42.963);
    expect(roundCoord(-85.6681234, 2)).toBe(-85.67);
  });

  it("rounds to nearest rather than truncating", () => {
    expect(roundCoord(42.999, 2)).toBe(43);
    expect(roundCoord(42.994, 2)).toBe(42.99);
    expect(roundCoord(0.005, 2)).toBe(0.01);
  });

  it("leaves an already-rounded value untouched", () => {
    expect(roundCoord(38.9, 2)).toBe(38.9);
    expect(roundCoord(-77, 2)).toBe(-77);
    expect(roundCoord(0, 2)).toBe(0);
  });

  it("keeps the result URL-safe, including the negative-zero case", () => {
    // -0.0001 rounds to -0; both String() and JSON.stringify() render it "0",
    // so a URL path or a query key can never receive "-0".
    const urlPart = String(roundCoord(-0.0001, 2));
    expect(urlPart).toBe("0");
    expect(JSON.stringify(roundCoord(-0.0001, 2))).toBe("0");
  });
});

describe("roundCoords", () => {
  it("rounds both axes to the same precision", () => {
    expect(roundCoords(42.9634567, -85.6681234, PRECISION.WEATHER)).toEqual({
      lat: 42.96,
      lon: -85.67,
    });
  });

  it("keeps alert precision finer than weather precision", () => {
    const weather = roundCoords(42.9634567, -85.6681234, PRECISION.WEATHER);
    const point = roundCoords(42.9634567, -85.6681234, PRECISION.POINT);
    expect(point.lat).toBe(42.963);
    expect(point.lon).toBe(-85.668);
    expect(PRECISION.POINT).toBeGreaterThan(PRECISION.WEATHER);
    expect(weather.lat).not.toBe(point.lat);
  });
});

describe("roundNullableCoords", () => {
  it("is null until both axes are known", () => {
    expect(roundNullableCoords(null, -85.66, 2)).toBeNull();
    expect(roundNullableCoords(42.96, undefined, 2)).toBeNull();
    expect(roundNullableCoords(null, null, 2)).toBeNull();
  });

  it("rounds when both axes are present", () => {
    expect(roundNullableCoords(42.9634, -85.6681, 2)).toEqual({ lat: 42.96, lon: -85.67 });
  });
});

describe("locationKey", () => {
  it("is stable across GPS jitter inside the same rounded cell", () => {
    // The regression this guards: every metre of movement used to create a new
    // react-query entry, and so a new request.
    const a = locationKey(42.963401, -85.668101, PRECISION.WEATHER);
    const b = locationKey(42.963499, -85.668199, PRECISION.WEATHER);
    const c = locationKey(42.963777, -85.668311, PRECISION.WEATHER);
    expect(a).toBe("42.96,-85.67");
    expect(b).toBe(a);
    expect(c).toBe(a);
  });

  it("changes once the rounded cell changes", () => {
    expect(locationKey(42.9634, -85.6681, PRECISION.WEATHER)).toBe("42.96,-85.67");
    // Ends of the same 0.01-degree cell still collapse.
    expect(locationKey(42.9649, -85.6681, PRECISION.WEATHER)).toBe("42.96,-85.67");
    // One cell over is a new key.
    expect(locationKey(42.9651, -85.6681, PRECISION.WEATHER)).toBe("42.97,-85.67");
  });

  it("distinguishes more positions at alert precision than at weather precision", () => {
    const a = locationKey(42.9634, -85.6681, PRECISION.POINT);
    const b = locationKey(42.9639, -85.6681, PRECISION.POINT);
    // 3 decimals separates these two; 2 decimals already collapses them, which
    // is why alerts get the finer value.
    expect(a).toBe("42.963,-85.668");
    expect(b).toBe("42.964,-85.668");
    expect(locationKey(42.9639, -85.6681, PRECISION.WEATHER)).toBe("42.96,-85.67");
  });
});
