import { sampleWindField, type WindField } from "../../src/hooks/useWindField";
import { sampleWind } from "../../../web/src/shared/windSample";

jest.mock("../../src/stores/useWeatherStore", () => ({}));

const field: WindField = {
  ok: true,
  timestamp: "2026-10-30T12:00:00Z",
  width: 2,
  height: 2,
  lat_min: 40,
  lat_max: 41,
  lon_min: -90,
  lon_max: -89,
  u_min: -10,
  u_max: 10,
  v_min: 0,
  v_max: 20,
  u: [-127, 127, -127, 127],
  v: [-127, -127, 127, 127],
};

describe("sampleWindField", () => {
  it("samples a grid point and interpolates between cells", () => {
    const [u, v] = sampleWindField(field, 40.5, -89.5);
    expect(u).toBeCloseTo(0, 6);
    expect(v).toBeCloseTo(10, 6);
    expect(sampleWindField(field, 41, -90)).toEqual([-10, 0]);
    expect(sampleWindField(field, 40, -89)).toEqual([10, 20]);
  });

  it("holds particles still (0, 0) outside the field or the model domain", () => {
    expect(sampleWindField(field, 42, -89.5)).toEqual([0, 0]);
    expect(sampleWindField({ ...field, u: [-128, 127, -127, 127] }, 40.5, -89.5)).toEqual([0, 0]);
    expect(sampleWindField({ ...field, v: [-127, -127, -127, -128] }, 40.5, -89.5)).toEqual([0, 0]);
    expect(sampleWindField(field, Number.NaN, -89.5)).toEqual([0, 0]);
  });
});

// The app keeps its own self-contained worklet sampler (a call into the shared one is the suspected
// Air Quality crash). It must stay numerically identical to the shared sampler the web radar uses,
// with the shared sampler's null mapped to (0, 0).
describe("sampleWindField parity with the shared sampler", () => {
  // Small deterministic PRNG so failures reproduce.
  function rng(seed: number) {
    let s = seed >>> 0;
    return () => {
      s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
      return s / 0x100000000;
    };
  }

  function makeField(seed: number, width: number, height: number, holes: boolean): WindField {
    const next = rng(seed);
    const cell = (): number => {
      if (holes && next() < 0.15) return -128;
      return Math.floor(next() * 255) - 127; // -127..127
    };
    const n = width * height;
    return {
      ok: true,
      timestamp: "2026-10-30T12:00:00Z",
      width,
      height,
      lat_min: 21.1,
      lat_max: 52.4,
      lon_min: -134.1,
      lon_max: -60.9,
      u_min: -41.3,
      u_max: 47.9,
      v_min: -38.6,
      v_max: 44.2,
      u: Array.from({ length: n }, cell),
      v: Array.from({ length: n }, cell),
    };
  }

  const expected = (f: WindField, lat: number, lon: number) => sampleWind(f, lat, lon) ?? [0, 0];

  it.each([
    ["dense, no holes", 11, 40, 30, false],
    ["dense, with -128 cells", 12, 40, 30, true],
    ["tiny grid, with -128 cells", 13, 3, 2, true],
    ["single row", 14, 5, 1, false],
  ])("agrees on random points: %s", (_name, seed, width, height, holes) => {
    const f = makeField(seed, width, height, holes);
    const next = rng(seed * 7919);
    let inside = 0;
    let zeroed = 0;
    for (let i = 0; i < 4000; i++) {
      // Roughly 20% of the samples fall outside the field on each axis.
      const lat = f.lat_min - 0.1 * (f.lat_max - f.lat_min) + next() * 1.2 * (f.lat_max - f.lat_min);
      const lon = f.lon_min - 0.1 * (f.lon_max - f.lon_min) + next() * 1.2 * (f.lon_max - f.lon_min);
      const got = sampleWindField(f, lat, lon);
      expect(got).toEqual(expected(f, lat, lon));
      if (sampleWind(f, lat, lon)) inside++;
      else zeroed++;
    }
    // The sweep must exercise both the interpolated and the (0, 0) paths.
    expect(inside).toBeGreaterThan(500);
    expect(zeroed).toBeGreaterThan(100);
  });

  it("agrees on every grid node, edge midpoint and the four corners", () => {
    const f = makeField(21, 9, 7, true);
    for (let y = 0; y < f.height; y++) {
      for (let x = 0; x < f.width; x++) {
        for (const [ox, oy] of [[0, 0], [0.5, 0], [0, 0.5], [0.5, 0.5], [0.999, 0.999]]) {
          const fx = Math.min(f.width - 1, x + ox);
          const fy = Math.min(f.height - 1, y + oy);
          const lon = f.lon_min + (fx / (f.width - 1)) * (f.lon_max - f.lon_min);
          const lat = f.lat_max - (fy / (f.height - 1)) * (f.lat_max - f.lat_min);
          expect(sampleWindField(f, lat, lon)).toEqual(expected(f, lat, lon));
        }
      }
    }
  });

  it("agrees just outside each edge, on the edge, and for non-finite input", () => {
    const f = makeField(31, 8, 6, false);
    const eps = 1e-9;
    const lats = [f.lat_min - eps, f.lat_min, f.lat_max, f.lat_max + eps, (f.lat_min + f.lat_max) / 2];
    const lons = [f.lon_min - eps, f.lon_min, f.lon_max, f.lon_max + eps, (f.lon_min + f.lon_max) / 2];
    for (const lat of [...lats, Number.NaN, Infinity, -Infinity]) {
      for (const lon of [...lons, Number.NaN, Infinity, -Infinity]) {
        expect(sampleWindField(f, lat, lon)).toEqual(expected(f, lat, lon));
      }
    }
  });

  it("maps a field made entirely of -128 cells to (0, 0)", () => {
    const f = { ...makeField(41, 4, 4, false), u: new Array(16).fill(-128), v: new Array(16).fill(-128) };
    expect(sampleWindField(f, 35, -100)).toEqual([0, 0]);
    expect(sampleWind(f, 35, -100)).toBeNull();
  });
});
