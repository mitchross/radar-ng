import { describe, expect, it } from "vitest";
import { SATELLITE_MAX_AGE_S, satelliteFrames, satelliteIndexAt, satelliteSequence, satelliteTileUrl } from "./shared/satellite";

const NOW = 1_800_000_000;
const iso = (s: number) => new Date(s * 1000).toISOString().replace(".000Z", "+00:00");
const path = (s: number) => new Date(s * 1000).toISOString().replace(".000Z", "Z");
// Six frames, 10 min apart, the newest 40 min before NOW (the GIBS latency).
const times = Array.from({ length: 6 }, (_, i) => NOW - 40 * 60 - (5 - i) * 600);
const manifest = {
  layers: {
    radar: { frames: [{ timestamp: iso(NOW), path: iso(NOW) }] },
    satellite: { frames: times.map((t) => ({ timestamp: iso(t), path: path(t), max_zoom: 7 })) },
  },
};

describe("satelliteFrames", () => {
  it("reads the manifest layer oldest first", () => {
    const frames = satelliteFrames(manifest);
    expect(frames.map((f) => f.time)).toEqual(times);
    expect(frames[0]).toMatchObject({ path: path(times[0]), maxZoom: 7 });
  });
  it("is empty without the layer", () => {
    expect(satelliteFrames({ layers: {} })).toEqual([]);
    expect(satelliteFrames(null)).toEqual([]);
  });
});

describe("satelliteIndexAt", () => {
  const frames = satelliteFrames(manifest);
  it("picks the frame at or before the timeline time", () => {
    expect(satelliteIndexAt(frames, times[2])).toBe(2);
    expect(satelliteIndexAt(frames, times[2] + 599)).toBe(2);
    expect(satelliteIndexAt(frames, NOW)).toBe(5);
  });
  it("hides before the first frame and once the newest is too old", () => {
    expect(satelliteIndexAt(frames, times[0] - 1)).toBe(-1);
    expect(satelliteIndexAt(frames, times[5] + SATELLITE_MAX_AGE_S)).toBe(5);
    expect(satelliteIndexAt(frames, times[5] + SATELLITE_MAX_AGE_S + 1)).toBe(-1);
  });
});

describe("satelliteSequence", () => {
  it("lists each satellite frame once, in playback order", () => {
    const frames = satelliteFrames(manifest);
    // Radar every 2 min from the oldest satellite frame to +1 h.
    const timeline = Array.from({ length: 60 }, (_, i) => times[0] + i * 120);
    expect(satelliteSequence(frames, timeline)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(satelliteSequence(frames, [NOW + 6 * 3600])).toEqual([]);
  });
});

describe("satelliteTileUrl", () => {
  it("builds the proxied tile template", () => {
    expect(satelliteTileUrl("https://radar.example", "2026-10-15T22:10:00Z")).toBe(
      "https://radar.example/tiles/satellite/2026-10-15T22:10:00Z/{z}/{x}/{y}.png",
    );
  });
});
