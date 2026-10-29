import { withClientAge, type LightningCollection } from "../../src/hooks/useLightning";

jest.mock("../../src/lib/storage", () => ({
  getString: jest.fn((_k: string, d: string) => d),
  setString: jest.fn(),
}));

const strike = (time: number, age_s: number) => ({
  type: "Feature" as const,
  geometry: { type: "Point" as const, coordinates: [-97, 35] as [number, number] },
  properties: { time, age_s, polarity: -1, mds: 0 },
});

describe("withClientAge", () => {
  it("ignores the server's frozen age_s and ages strikes from their own time", () => {
    const now = 1_800_000_000;
    // Worker stalled: the file still says these strikes are 5 s old.
    const fc: LightningCollection = {
      type: "FeatureCollection",
      generated_at: now - 600,
      features: [strike(now - 605, 5), strike(now - 20, 5), strike(now + 3, 5)],
    };
    const ages = withClientAge(fc, now).features.map((f) => f.properties.age_s);
    expect(ages).toEqual([605, 20, 0]);
  });
});
