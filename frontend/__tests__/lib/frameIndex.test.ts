import { findClosestIdx } from "../../src/lib/frameIndex";

const frames = [{ time: 100 }, { time: 200 }, { time: 300 }];

describe("findClosestIdx", () => {
  it("returns 0 for an empty frame list instead of crashing", () => {
    // Regression: the timeline's segment-boundary useMemo runs before the
    // component's empty-frames early return, so this is called with [] on first
    // load. It must not dereference frames[-1].
    expect(() => findClosestIdx([], 12345)).not.toThrow();
    expect(findClosestIdx([], 12345)).toBe(0);
  });

  it("finds the nearest frame by time", () => {
    expect(findClosestIdx(frames, 90)).toBe(0);
    expect(findClosestIdx(frames, 210)).toBe(1);
    expect(findClosestIdx(frames, 260)).toBe(2);
  });

  it("clamps to the last frame when the target is past the end", () => {
    expect(findClosestIdx(frames, 999)).toBe(2);
  });
});
