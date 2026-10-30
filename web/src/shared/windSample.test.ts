import { describe, expect, it } from "vitest";
import { sampleWind, type WindField } from "./windSample";

// 2 x 2 grid over lat 40..41, lon -90..-89. u is stored -127..127 over [-10, 10] mph, v over [0, 20].
const field = (over: Partial<WindField> = {}): WindField => ({
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
  ...over,
});

describe("sampleWind", () => {
  it("returns the cell value at a grid corner", () => {
    // North-west corner is index 0: u at its minimum, v at its minimum.
    expect(sampleWind(field(), 41, -90)).toEqual([-10, 0]);
    // South-east corner is index 3: both at their maximum.
    expect(sampleWind(field(), 40, -89)).toEqual([10, 20]);
  });

  it("interpolates bilinearly between cells", () => {
    const [u, v] = sampleWind(field(), 40.5, -89.5)!;
    expect(u).toBeCloseTo(0, 6);
    expect(v).toBeCloseTo(10, 6);
  });

  it("returns null outside the field", () => {
    expect(sampleWind(field(), 42, -89.5)).toBeNull();
    expect(sampleWind(field(), 40.5, -91)).toBeNull();
    expect(sampleWind(field(), Number.NaN, -89.5)).toBeNull();
  });

  it("returns null when any neighbouring cell is outside the model domain", () => {
    expect(sampleWind(field({ u: [-128, 127, -127, 127] }), 40.5, -89.5)).toBeNull();
    expect(sampleWind(field({ v: [-127, -127, -127, -128] }), 40.5, -89.5)).toBeNull();
  });
});
