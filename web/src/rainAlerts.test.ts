import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BrowserRainAlerts } from "./rainAlerts";
import { planRainAlert } from "./shared/rainAlerts";
import { describeNowcastSkill } from "./shared/nowcastSkill";
import type { RadarNowcastResponse } from "./shared/nowcastTypes";

const NOW = Date.UTC(2026, 8, 30, 15);
const MIN = 60_000;
const delivered = vi.fn();
const requestPermission = vi.fn();
const nowcast: RadarNowcastResponse = {
  status: "ok",
  issued_at: new Date(NOW - 4 * MIN).toISOString(),
  points: Array.from({ length: 12 }, (_, i) => ({
    timestamp: new Date(NOW - 4 * MIN + (i + 1) * 5 * MIN).toISOString(),
    lead_minutes: (i + 1) * 5,
    precipitation_mm_h: i >= 3 && i <= 5 ? 5 : 0,
    dbz: i >= 3 && i <= 5 ? 25 : 0,
  })),
};

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  delivered.mockClear();
  requestPermission.mockReset().mockResolvedValue("granted");
  class MockNotification {
    static permission = "granted";
    static requestPermission = requestPermission;
    constructor(title: string, options: NotificationOptions) { delivered(title, options); }
  }
  vi.stubGlobal("Notification", MockNotification);
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("shows the same accuracy headline as the app and withholds it during warmup", () => {
  const skill = { pod: 0.87, far: 0.09, csi: 0.8, persistence_csi: 0.6, runs: 6, lead_minutes: 30, window_hours: 24 };
  expect(describeNowcastSkill(skill)?.sentence).toBe("Caught 87% of rain 30 min out today, 9% false alarms");
  expect(describeNowcastSkill({ ...skill, runs: 5 })).toBeNull();
});

it("requests permission only when the user enables alerts", async () => {
  Object.defineProperty(Notification, "permission", { value: "default", configurable: true });
  const alerts = new BrowserRainAlerts();
  expect(requestPermission).not.toHaveBeenCalled();
  await alerts.setEnabled(true);
  expect(requestPermission).toHaveBeenCalledOnce();
  expect(alerts.enabled).toBe(true);
});

it("keeps alerts disabled when permission is denied", async () => {
  Object.defineProperty(Notification, "permission", { value: "denied", configurable: true });
  requestPermission.mockResolvedValue("denied");
  const alerts = new BrowserRainAlerts();
  await alerts.setEnabled(true);
  alerts.sync(nowcast, "Boston");
  vi.runAllTimers();
  expect(alerts.enabled).toBe(false);
  expect(delivered).not.toHaveBeenCalled();
});

it("replaces the pending plan across new runs and only delivers once per storm", async () => {
  const alerts = new BrowserRainAlerts();
  await alerts.setEnabled(true);
  alerts.sync(nowcast, "Boston");
  const refreshed = { ...nowcast, issued_at: new Date(NOW - 2 * MIN).toISOString() };
  alerts.sync(refreshed, "Boston");
  const plan = planRainAlert(refreshed, "Boston", { leadMinutes: 10 }, NOW)!;
  vi.advanceTimersByTime(plan.fireAt - NOW);
  expect(delivered).toHaveBeenCalledOnce();
  expect(delivered.mock.calls[0][1].body).toContain("in about 10 min");
  alerts.sync(refreshed, "Boston");
  vi.runAllTimers();
  expect(delivered).toHaveBeenCalledOnce();
});

it("cancels the notification when rain disappears or the user disables alerts", async () => {
  const alerts = new BrowserRainAlerts();
  await alerts.setEnabled(true);
  alerts.sync(nowcast, "Boston");
  alerts.sync({ ...nowcast, points: nowcast.points.map((p) => ({ ...p, precipitation_mm_h: 0 })) }, "Boston");
  vi.runAllTimers();
  expect(delivered).not.toHaveBeenCalled();
  alerts.sync(nowcast, "Boston");
  await alerts.setEnabled(false);
  vi.runAllTimers();
  expect(delivered).not.toHaveBeenCalled();
});

it("reschedules when the warning lead changes", async () => {
  const alerts = new BrowserRainAlerts();
  await alerts.setEnabled(true);
  alerts.sync(nowcast, "Boston");
  alerts.setLead("5");
  alerts.sync(nowcast, "Boston");
  const plan = planRainAlert(nowcast, "Boston", { leadMinutes: 5 }, NOW)!;
  vi.advanceTimersByTime(plan.fireAt - NOW - 1);
  expect(delivered).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(delivered).toHaveBeenCalledOnce();
});
