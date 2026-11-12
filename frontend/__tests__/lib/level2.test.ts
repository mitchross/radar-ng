import { closeUpRadarFrames, closeUpAxisLabels, closeUpHeader, type Level2View } from "../../src/lib/level2";
import type { RadarFrame } from "../../src/types/weather";

const l2 = (time: number) => ({ time, timestamp: `t${time}`, path: `p${time}`, scanEndMs: time * 1000 });
const view = { frames: [l2(1000), l2(2000)] } as unknown as Level2View;
const mrms = (time: number): RadarFrame => ({ time, timestamp: `m${time}`, path: `m${time}`, source: "radar" });

describe("closeUpRadarFrames", () => {
  it("keeps each sweep's own time and path, over an MRMS frame or an explicit gap", () => {
    const out = closeUpRadarFrames(view, [mrms(1000)]);
    expect(out.map((f) => f.time)).toEqual([1000, 2000]);
    expect(out.every((f) => f.level2)).toBe(true);
    expect(out[0].gap).toBeFalsy();
    expect(out[0].path).toBe("m1000");
    expect(out[1].gap).toBe(true);
  });

  it("labels the header by scan time, never Now", () => {
    const [f] = closeUpRadarFrames(view, [mrms(1000)]);
    const h = closeUpHeader(f, "UTC");
    expect(h.title).toBe("Radar · Level II close-up");
    expect(h.dateLabel.startsWith("Scan ")).toBe(true);
    expect(closeUpAxisLabels([1000, 2000], "UTC")).toHaveLength(5);
    expect(closeUpAxisLabels([])).toEqual([]);
  });
});
