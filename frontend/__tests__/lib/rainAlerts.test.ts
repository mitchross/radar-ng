import {
  intensityAt,
  parseLastRainAlert,
  parseRainAlertLead,
  planRainAlert,
  shouldNotify,
  winterStart,
} from "../../src/lib/rainAlerts";
import type { RadarNowcastResponse } from "../../src/types/weather";

const NOW = Date.UTC(2026, 8, 30, 15, 0, 0);
const MIN = 60_000;

function nowcast(mmPerHourByLead: Record<number, number>, issuedAt = NOW - 4 * MIN): RadarNowcastResponse {
  return {
    status: "ok",
    issued_at: new Date(issuedAt).toISOString(),
    points: Object.entries(mmPerHourByLead).map(([lead, mm]) => ({
      timestamp: new Date(issuedAt + Number(lead) * MIN).toISOString(),
      lead_minutes: Number(lead),
      dbz: mm > 0 ? 25 : null,
      precipitation_mm_h: mm,
    })),
  };
}

const dry = { 5: 0, 10: 0, 15: 0, 20: 0, 25: 0, 30: 0, 35: 0, 40: 0, 45: 0, 50: 0, 55: 0, 60: 0 };

describe("planRainAlert", () => {
  it("says nothing while it stays dry", () => {
    expect(planRainAlert(nowcast(dry), "Boston", { leadMinutes: 10 }, NOW)).toBeNull();
  });

  it("says nothing when it is already raining", () => {
    expect(planRainAlert(nowcast({ ...dry, 5: 6, 10: 6, 15: 6 }), "Boston", { leadMinutes: 10 }, NOW)).toBeNull();
  });

  it("plans a notification ahead of the first wet minute", () => {
    // 2.54 mm/h = 0.1 in/hr from +20 (issued 4 min ago => starts ~16 min from now).
    const plan = planRainAlert(
      nowcast({ ...dry, 20: 2.54, 25: 2.54, 30: 2.54, 35: 0 }),
      "Boston",
      { leadMinutes: 10 },
      NOW,
    );
    expect(plan).not.toBeNull();
    const startInMin = Math.round((plan!.startAt - NOW) / MIN);
    expect(startInMin).toBeGreaterThanOrEqual(12);
    expect(startInMin).toBeLessThanOrEqual(16);
    expect(plan!.fireAt).toBe(plan!.startAt - 10 * MIN);
    expect(plan!.heavy).toBe(false);
    expect(plan!.title).toMatch(/^Rain starting around /);
    expect(plan!.body).toMatch(/reaching Boston in about \d+ min, lasting about \d+ min\./);
    expect(plan!.endAt).not.toBeNull();
  });

  it("fires right away when rain starts inside the lead time", () => {
    const plan = planRainAlert(nowcast({ ...dry, 10: 5, 15: 5 }), "", { leadMinutes: 15 }, NOW)!;
    expect(plan.fireAt).toBe(NOW + 1_000);
    expect(plan.body).toContain("your location");
  });

  it("calls a heavy peak heavy and leaves the end open past the horizon", () => {
    const plan = planRainAlert(
      nowcast({ ...dry, 30: 10, 35: 12, 40: 12, 45: 12, 50: 12, 55: 12, 60: 12 }),
      "Boston",
      { leadMinutes: 5 },
      NOW,
    )!;
    expect(plan.heavy).toBe(true);
    expect(plan.title).toMatch(/^Heavy rain/);
    expect(plan.endAt).toBeNull();
    expect(plan.body).not.toContain("lasting");
  });

  it("ignores unusable nowcasts", () => {
    expect(planRainAlert(undefined, "x", { leadMinutes: 10 }, NOW)).toBeNull();
    expect(planRainAlert({ ...nowcast(dry), status: "unavailable" }, "x", { leadMinutes: 10 }, NOW)).toBeNull();
    expect(planRainAlert({ status: "ok", points: [] }, "x", { leadMinutes: 10 }, NOW)).toBeNull();
  });

  it("keeps a stable key for the same storm across refreshes", () => {
    const a = planRainAlert(nowcast({ ...dry, 30: 5, 35: 5 }), "x", { leadMinutes: 10 }, NOW)!;
    const b = planRainAlert(nowcast({ ...dry, 30: 5, 35: 5 }), "x", { leadMinutes: 10 }, NOW + 2 * MIN)!;
    expect(a.key).toBe(b.key);
  });
});

describe("intensityAt", () => {
  const series = [
    { at: NOW, inHr: 0 },
    { at: NOW + 10 * MIN, inHr: 1 },
  ];
  it("interpolates linearly and is unknown past the last sample", () => {
    expect(intensityAt(series, NOW + 5 * MIN)).toBeCloseTo(0.5);
    expect(intensityAt(series, NOW + 11 * MIN)).toBeNull();
    expect(intensityAt(series, NOW - 2 * MIN)).toBe(0);
    expect(intensityAt(series, NOW - 6 * MIN)).toBeNull();
    expect(intensityAt([], NOW)).toBeNull();
  });
});

describe("shouldNotify", () => {
  const plan = planRainAlert(nowcast({ ...dry, 30: 5, 35: 5 }), "x", { leadMinutes: 10 }, NOW)!;
  it("announces a new event, and the same event again only after a long gap", () => {
    expect(shouldNotify(plan, null, NOW)).toBe(true);
    expect(shouldNotify(plan, { key: "old", startAt: plan.startAt + 3 * MIN, firedAt: NOW - 20 * MIN }, NOW)).toBe(false);
    expect(shouldNotify(plan, { key: "old", startAt: plan.startAt + 30 * MIN, firedAt: NOW - 20 * MIN }, NOW)).toBe(true);
    expect(shouldNotify(plan, { key: "old", startAt: plan.startAt, firedAt: NOW - 2 * 60 * MIN }, NOW)).toBe(true);
  });
});

describe("persisted prefs", () => {
  it("parses stored values defensively", () => {
    expect(parseLastRainAlert("")).toBeNull();
    expect(parseLastRainAlert("{bad")).toBeNull();
    expect(parseLastRainAlert(JSON.stringify({ key: "k", startAt: 1, firedAt: 2 }))).toEqual({ key: "k", startAt: 1, firedAt: 2 });
    expect(parseRainAlertLead("15")).toBe(15);
    expect(parseRainAlertLead("7")).toBe(10);
    expect(parseRainAlertLead("")).toBe(10);
  });
});

describe("snow start", () => {
  // 0.4 mm/h is about 16 dBZ: under the rain start, over the snow start.
  const light = { ...dry, 25: 0.4, 30: 0.4, 35: 0.4 };
  const classed = (cls: "snow" | "rain"): RadarNowcastResponse => {
    const body = nowcast(light);
    return { ...body, points: body.points.map((p) => ({ ...p, precip_class: cls })) };
  };
  it("notifies light snow the rain rule would skip, counting from the notification", () => {
    const plan = planRainAlert(classed("snow"), "Detroit", { leadMinutes: 10 }, NOW)!;
    expect(plan.precipClass).toBe("snow");
    expect(plan.title).toBe("Snow starts in ~10 min");
    expect(winterStart(classed("snow"), NOW)?.words).toMatch(/^Snow starts in ~\d+ min$/);
  });
  it("keeps rain unchanged without a class or with rain", () => {
    expect(planRainAlert(nowcast(light), "Detroit", { leadMinutes: 10 }, NOW)).toBeNull();
    expect(planRainAlert(classed("rain"), "Detroit", { leadMinutes: 10 }, NOW)).toBeNull();
    expect(winterStart(classed("rain"), NOW)).toBeNull();
  });
});
