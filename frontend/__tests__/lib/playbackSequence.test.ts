import { offsetLabel, playbackSequence, positionOf } from "../../src/lib/playbackSequence";

const NOW = 1_800_000_000;
// 4h of MRMS every 2 min, nowcast every 5 min to +60, HRRR hourly +2h..+17h.
const frames = [
  ...Array.from({ length: 121 }, (_, i) => ({ time: NOW - (120 - i) * 120 })),
  ...Array.from({ length: 12 }, (_, i) => ({ time: NOW + (i + 1) * 300 })),
  ...Array.from({ length: 16 }, (_, i) => ({ time: NOW + (i + 2) * 3600 })),
];

describe("playbackSequence", () => {
  it("1h visits every frame within an hour of now, in order", () => {
    const seq = playbackSequence(frames, "1h", NOW);
    expect(seq.map((i) => frames[i].time - NOW)).toEqual([
      ...Array.from({ length: 31 }, (_, i) => -3600 + i * 120),
      ...Array.from({ length: 12 }, (_, i) => (i + 1) * 300),
    ]);
  });

  it("48h thins the past to 15 minutes, keeps the latest past frame and every future frame", () => {
    const seq = playbackSequence(frames, "48h", NOW);
    const offsets = seq.map((i) => frames[i].time - NOW);
    const past = offsets.filter((o) => o <= 0);
    expect(past[past.length - 1]).toBe(0);
    for (let k = 1; k < past.length; k++) expect(past[k] - past[k - 1]).toBeGreaterThanOrEqual(900);
    expect(past.length).toBeLessThanOrEqual(17);
    expect(offsets.filter((o) => o > 0)).toHaveLength(28);
    // Strictly increasing, so a +1 step always moves forward in time.
    for (let k = 1; k < seq.length; k++) expect(seq[k]).toBeGreaterThan(seq[k - 1]);
    // Most of the loop is forecast.
    expect(offsets.filter((o) => o > 0).length).toBeGreaterThan(past.length);
  });

  it("falls back to the nearest frame when nothing is within an hour", () => {
    expect(playbackSequence([{ time: NOW - 9000 }, { time: NOW - 8000 }], "1h", NOW)).toEqual([1]);
    expect(playbackSequence([], "48h", NOW)).toEqual([]);
  });
});

describe("positionOf", () => {
  it("finds exact members and snaps skipped frames to the nearest in time", () => {
    const seq = playbackSequence(frames, "48h", NOW);
    expect(positionOf(seq, frames, seq[3])).toBe(3);
    const skipped = seq[3] + 1; // 2 min after a kept frame
    expect(positionOf(seq, frames, skipped)).toBe(3);
  });
});

describe("offsetLabel", () => {
  it("formats minutes under 90 and hours beyond", () => {
    expect(offsetLabel(NOW, NOW)).toBe("Now");
    expect(offsetLabel(NOW + 45 * 60, NOW)).toBe("+45m");
    expect(offsetLabel(NOW - 4 * 3600, NOW)).toBe("−4h");
    expect(offsetLabel(NOW + 17 * 3600, NOW)).toBe("+17h");
  });
});
