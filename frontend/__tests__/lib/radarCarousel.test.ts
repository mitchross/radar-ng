import { assignSlots, clampWindow } from "../../src/lib/radarCarousel";

describe("assignSlots", () => {
  const N = 5;

  it("assigns the next N frames in playback order starting at current", () => {
    const { slots, visibleSlot } = assignSlots(10, 10, 30, N);
    // curP = 0 → slots hold frames 10..14, slot p%N
    expect(slots).toEqual([10, 11, 12, 13, 14]);
    expect(visibleSlot).toBe(0);
  });

  it("advancing by one reassigns exactly one slot", () => {
    const before = assignSlots(12, 0, 20, N).slots;
    const after = assignSlots(13, 0, 20, N).slots;
    const changed = before.filter((f, s) => after[s] !== f);
    expect(changed).toHaveLength(1);
    // the slot that just played (frame 12) now prefetches 12 + N
    expect(after[before.indexOf(12)]).toBe(12 + N);
  });

  it("keeps the current frame mounted in the same slot across every advance", () => {
    // Walk an entire loop of a window whose length is not a multiple of N.
    const start = 3;
    const end = 9; // winLen = 7
    let prev = assignSlots(start, start, end, N);
    for (let step = 0; step < 20; step++) {
      const idx = start + ((step + 1) % (end - start + 1));
      const cur = assignSlots(idx, start, end, N);
      // The frame about to become visible was already assigned to that
      // slot in the previous tick (i.e. it was prefetching while hidden).
      expect(prev.slots[cur.visibleSlot]).toBe(idx);
      expect(cur.slots[cur.visibleSlot]).toBe(idx);
      prev = cur;
    }
  });

  it("prefetches the loop wrap: last window frame's successors alias to window start", () => {
    const { slots, visibleSlot } = assignSlots(20, 11, 20, N); // winLen 10, curP 9
    expect(slots[visibleSlot]).toBe(20);
    // remaining slots hold the wrap frames from the window start
    const others = slots.filter((_, s) => s !== visibleSlot).sort((a, b) => a - b);
    expect(others).toEqual([11, 12, 13, 14]);
  });

  it("handles windows shorter than the slot count with duplicates, never gaps", () => {
    const { slots, visibleSlot } = assignSlots(1, 0, 2, N); // winLen 3 < N
    expect(slots).toHaveLength(N);
    for (const f of slots) {
      expect(f).toBeGreaterThanOrEqual(0);
      expect(f).toBeLessThanOrEqual(2);
    }
    expect(slots[visibleSlot]).toBe(1);
  });

  it("handles a single-frame window", () => {
    const { slots, visibleSlot } = assignSlots(0, 0, 0, N);
    expect(slots).toEqual([0, 0, 0, 0, 0]);
    expect(visibleSlot).toBe(0);
  });

  it("widens the window when the current frame falls outside it", () => {
    const below = assignSlots(2, 5, 15, N);
    expect(below.slots[below.visibleSlot]).toBe(2);
    const above = assignSlots(18, 5, 15, N);
    expect(above.slots[above.visibleSlot]).toBe(18);
  });

  it("scrub jumps still keep the visible frame assigned", () => {
    for (const idx of [0, 7, 19, 3, 11]) {
      const { slots, visibleSlot } = assignSlots(idx, 0, 19, N);
      expect(slots[visibleSlot]).toBe(idx);
    }
  });

  it("WINDOW=1 kill switch degenerates to the single current frame", () => {
    const { slots, visibleSlot } = assignSlots(4, 0, 10, 1);
    expect(slots).toEqual([4]);
    expect(visibleSlot).toBe(0);
  });
});

describe("clampWindow", () => {
  it("falls back to the full range when null", () => {
    expect(clampWindow(null, 12)).toEqual({ start: 0, end: 11 });
  });

  it("clamps a stale window after the frame list shrank", () => {
    expect(clampWindow({ start: 5, end: 40 }, 10)).toEqual({ start: 5, end: 9 });
    expect(clampWindow({ start: 20, end: 40 }, 10)).toEqual({ start: 9, end: 9 });
  });

  it("passes a valid window through unchanged", () => {
    expect(clampWindow({ start: 2, end: 6 }, 10)).toEqual({ start: 2, end: 6 });
  });
});
