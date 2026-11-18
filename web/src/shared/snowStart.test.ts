import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { PrecipClass, RadarNowcastResponse } from "./nowcastTypes";
import { SNOW_START_IN_HR, planRainAlert, winterStart } from "./rainAlerts";

type Fixture = {
  snow_onset: string;
  reflectivity: { time: string; dbz: number }[];
};
const kdca = JSON.parse(
  readFileSync(new URL("./testdata/snowStart.kdca-2025-01-06.json", import.meta.url), "utf8"),
) as Fixture;

const MIN = 60_000;
/** The server's Marshall-Palmer rate (radar-api `dbz_to_rain_rate`). */
const mmH = (dbz: number) => (dbz < 5 ? 0 : Math.min(300, ((10 ** (dbz / 10)) / 200) ** (1 / 1.6)));

/** The observed hour after `now` as a perfect nowcast, every 5 minutes. */
function replay(now: number, cls?: PrecipClass): RadarNowcastResponse {
  const obs = kdca.reflectivity.map((r) => ({ at: Date.parse(r.time), dbz: r.dbz }));
  const at = (t: number) => obs.reduce((best, o) => (Math.abs(o.at - t) < Math.abs(best.at - t) ? o : best));
  return {
    status: "ok",
    issued_at: new Date(now).toISOString(),
    points: Array.from({ length: 12 }, (_, i) => {
      const t = now + (i + 1) * 5 * MIN;
      const dbz = at(t).dbz;
      return {
        timestamp: new Date(t).toISOString(),
        lead_minutes: (i + 1) * 5,
        dbz,
        precipitation_mm_h: mmH(dbz),
        ...(cls ? { precip_class: cls } : {}),
      };
    }),
  };
}

const prefs = { leadMinutes: 10 } as const;

describe("snow start on the 2025-01-06 Washington snow", () => {
  const now = Date.parse("2025-01-06T05:20:00Z");
  const onset = Date.parse(kdca.snow_onset);

  it("puts the threshold at about 15 dBZ", () => {
    expect(mmH(14.5) / 25.4).toBeLessThan(SNOW_START_IN_HR);
    expect(mmH(15) / 25.4).toBeGreaterThanOrEqual(SNOW_START_IN_HR);
  });

  it("calls snow within 10 minutes of the reported onset", () => {
    const start = winterStart(replay(now, "snow"), now);
    expect(start?.precipClass).toBe("snow");
    expect(Math.abs(start!.startAt - onset)).toBeLessThanOrEqual(10 * MIN);
    expect(start!.words).toMatch(/^Snow starts in ~\d+ min$/);
  });

  it("does not call the 12-14 dBZ echoes before the onset", () => {
    const start = winterStart(replay(now, "snow"), now);
    expect(start!.startAt).toBeGreaterThan(Date.parse("2025-01-06T05:50:00Z"));
  });

  it("plans a snow notification with a minutes title", () => {
    const plan = planRainAlert(replay(now, "snow"), "Washington", prefs, now);
    expect(plan?.precipClass).toBe("snow");
    expect(plan!.heavy).toBe(false);
    expect(plan!.title).toBe("Snow starts in ~10 min");
    expect(plan!.body).toContain("snow reaching Washington");
  });

  it("keeps the rain rule when the server sends no class (old servers) or rain", () => {
    expect(planRainAlert(replay(now), "Washington", prefs, now)).toBeNull();
    expect(planRainAlert(replay(now, "rain"), "Washington", prefs, now)).toBeNull();
    expect(winterStart(replay(now), now)).toBeNull();
  });

  it("words mix and ice", () => {
    expect(winterStart(replay(now, "mix"), now)!.words).toMatch(/^Wintry mix starts in ~\d+ min$/);
    // Ice keeps the rain threshold, so these weak echoes do not count.
    expect(winterStart(replay(now, "ice"), now)).toBeNull();
  });

  it("says nothing once it is snowing", () => {
    const later = Date.parse("2025-01-06T06:10:00Z");
    expect(winterStart(replay(later, "snow"), later)).toBeNull();
  });
});
