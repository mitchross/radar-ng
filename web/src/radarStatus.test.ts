import { describe, expect, it } from "vitest";
import { observedFrameTime, radarNoticeCopy, radarStatus } from "./radarStatus";

const NOW_MS = 1_800_000_000_000;
const NOW = NOW_MS / 1000;
const iso = (s: number) => new Date(s * 1000).toISOString();

describe("radarStatus", () => {
  it("is LIVE up to 10 minutes and names the age after", () => {
    expect(radarStatus({ frameTimeSeconds: NOW - 600, refreshFailed: false, loading: false, nowMilliseconds: NOW_MS }).label).toBe("LIVE");
    const stale = radarStatus({ frameTimeSeconds: NOW - 660, refreshFailed: false, loading: false, nowMilliseconds: NOW_MS });
    expect(stale).toEqual({ label: "UPDATED 11M AGO", accessibilityLabel: "Radar updated 11 minutes ago", tone: "stale" });
    expect(radarStatus({ frameTimeSeconds: NOW - 3 * 3600, refreshFailed: false, loading: false, nowMilliseconds: NOW_MS }).label).toBe("UPDATED 3H AGO");
    expect(radarStatus({ frameTimeSeconds: NOW - 2 * 86400, refreshFailed: false, loading: false, nowMilliseconds: NOW_MS }).label).toBe("UPDATED 2D AGO");
  });

  it("is OFFLINE when a refresh failed but frames are still shown", () => {
    expect(radarStatus({ frameTimeSeconds: NOW - 60, refreshFailed: true, loading: false, nowMilliseconds: NOW_MS }).tone).toBe("offline");
  });

  it("is LOADING before the first manifest and UNAVAILABLE after a failure", () => {
    expect(radarStatus({ frameTimeSeconds: null, refreshFailed: false, loading: true }).label).toBe("LOADING");
    expect(radarStatus({ frameTimeSeconds: null, refreshFailed: true, loading: true }).label).toBe("UNAVAILABLE");
    expect(radarStatus({ frameTimeSeconds: null, refreshFailed: false, loading: false }).label).toBe("UNAVAILABLE");
  });
});

describe("radarNoticeCopy", () => {
  const status = (frameTimeSeconds: number | null, refreshFailed: boolean, loading: boolean) =>
    radarStatus({ frameTimeSeconds, refreshFailed, loading, nowMilliseconds: NOW_MS });

  it("shows nothing for live and stale radar", () => {
    expect(radarNoticeCopy(status(NOW, false, false), true)).toBeNull();
    expect(radarNoticeCopy(status(NOW - 3600, false, false), true)).toBeNull();
  });

  it("explains loading without a retry, and offers retry when offline or unavailable", () => {
    expect(radarNoticeCopy(status(null, false, true), false)).toMatchObject({ title: "Loading radar", retry: false });
    expect(radarNoticeCopy(status(null, true, false), false)).toMatchObject({ title: "Radar unavailable", retry: true });
    expect(radarNoticeCopy(status(NOW, true, false), true)).toMatchObject({ title: "Radar offline", retry: true });
  });
});

describe("observedFrameTime", () => {
  it("picks the newest observed frame at or before now, ignoring the shown layer", () => {
    const manifest = {
      layers: {
        radar: { frames: [iso(NOW - 240), iso(NOW - 120), iso(NOW + 120)].map((timestamp) => ({ timestamp, path: timestamp })) },
        temperature: { timestamps: [iso(NOW + 3600)] },
      },
    };
    expect(observedFrameTime(manifest, NOW)).toBe(NOW - 120);
  });

  it("falls back to composite, and is null without radar", () => {
    expect(observedFrameTime({ layers: { "radar-composite": { timestamps: [iso(NOW - 60)] } } }, NOW)).toBe(NOW - 60);
    expect(observedFrameTime({ layers: {} }, NOW)).toBeNull();
    expect(observedFrameTime(null, NOW)).toBeNull();
  });
});
