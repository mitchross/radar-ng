import { describe, expect, it } from "vitest";
import { skyTonightCardHtml } from "./skyTonightCard";
import { parseSkyTonight } from "./shared/skyTonight";

const base = {
  dateKey: "2026-11-17",
  lat: 42.3314,
  lon: -83.0458,
  hourly: null,
  clock: { timeZone: "America/Detroit", utcOffsetSeconds: -5 * 3600 },
  report: parseSkyTonight({
    status: "ok",
    passes: [{ start: "2026-11-17T23:47:42Z", max_at: "2026-11-17T23:50:20Z", end: "2026-11-17T23:50:53Z", max_elevation_deg: 21, start_azimuth_deg: 294, max_azimuth_deg: 345, end_azimuth_deg: 0 }],
  }),
};

describe("skyTonightCardHtml", () => {
  it("shows the full card in the evening, with the CelesTrak credit", () => {
    const html = skyTonightCardHtml({ ...base, nowMs: Date.parse("2026-11-17T21:00:00Z"), expanded: false });
    expect(html).toContain("LOOK UP TONIGHT");
    expect(html).toContain("6:47 PM · WNW to N · 21° high · 3 min");
    expect(html).toContain("Orbital elements: CelesTrak");
  });
  it("is only a link in the morning until it is opened", () => {
    const now = Date.parse("2026-11-17T15:00:00Z");
    expect(skyTonightCardHtml({ ...base, nowMs: now, expanded: false })).toContain('id="sky-tonight-open"');
    expect(skyTonightCardHtml({ ...base, nowMs: now, expanded: true })).toContain("Planets");
  });
  it("adds ECCC seeing and its credit when the server has them", () => {
    const nowMs = Date.parse("2026-11-17T21:00:00Z");
    const best = { start: "2026-11-18T03:00:00Z", end: "2026-11-18T07:00:00Z", score: 84 };
    const report = parseSkyTonight({ status: "ok", passes: [], stargazing: { best, hours: [{ at: best.start, score: 84, seeing: 4, transparency: 3 }] } });
    const html = skyTonightCardHtml({ ...base, report, nowMs, expanded: false });
    expect(html).toContain("Seeing Good · Transparency Medium");
    expect(html).toContain("Data Source: Environment and Climate Change Canada");
    expect(skyTonightCardHtml({ ...base, nowMs, expanded: false })).not.toContain("Environment and Climate Change Canada");
  });
});
