import { sampleWindField, type WindField } from "../../src/hooks/useWindField";

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
  it("samples through the shared sampler", () => {
    const [u, v] = sampleWindField(field, 40.5, -89.5);
    expect(u).toBeCloseTo(0, 6);
    expect(v).toBeCloseTo(10, 6);
  });

  it("holds particles still (0, 0) outside the field or the model domain", () => {
    expect(sampleWindField(field, 42, -89.5)).toEqual([0, 0]);
    expect(sampleWindField({ ...field, u: [-128, 127, -127, 127] }, 40.5, -89.5)).toEqual([0, 0]);
  });
});
