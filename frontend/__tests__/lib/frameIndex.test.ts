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

describe("resnapFrameIndex", () => {
  const { resnapFrameIndex } = jest.requireActual("../../src/lib/frameIndex") as typeof import("../../src/lib/frameIndex");
  const t = (...times: number[]) => times.map((time) => ({ time }));

  it("asks for a now-snap on empty frames or an explicit -1", () => {
    expect(resnapFrameIndex([], 3, 100)).toBe(-1);
    expect(resnapFrameIndex(t(100, 200), -1, 100)).toBe(-1);
  });

  it("keeps the index when nothing was shown yet and it is still in range", () => {
    expect(resnapFrameIndex(t(100, 200, 300), 1, null)).toBe(1);
    expect(resnapFrameIndex(t(100, 200), 5, null)).toBe(-1);
  });

  it("keeps the index when the frame at that index still has the same time", () => {
    expect(resnapFrameIndex(t(100, 200, 300), 1, 200)).toBe(1);
  });

  it("follows the frame by time when a poll prunes the head", () => {
    // Was at index 2 (time 300); head pruned so 300 is now index 1.
    expect(resnapFrameIndex(t(200, 300, 400), 2, 300)).toBe(1);
  });

  it("follows the frame by time when new frames are appended at the head", () => {
    expect(resnapFrameIndex(t(50, 100, 200, 300), 0, 100)).toBe(1);
  });

  it("snaps to the nearest frame within tolerance when the exact time is gone", () => {
    // Paused on the oldest 2-min MRMS frame (t=100) which got pruned; nearest is 220 (2 min later).
    expect(resnapFrameIndex(t(220, 340, 460), 0, 100, 15 * 60)).toBe(0);
  });

  it("asks for a now-snap when no frame is anywhere near the old time (layer switch)", () => {
    expect(resnapFrameIndex(t(10_000, 13_600), 0, 100, 15 * 60)).toBe(-1);
  });
});
