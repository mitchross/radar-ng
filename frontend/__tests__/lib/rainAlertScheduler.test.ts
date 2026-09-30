import * as Notifications from "expo-notifications";
import { syncRainAlert } from "../../src/lib/rainAlertScheduler";
import type { RainAlertPlan } from "../../src/lib/rainAlerts";

jest.mock("expo-notifications", () => ({
  AndroidImportance: { HIGH: 4 },
  SchedulableTriggerInputTypes: { DATE: "date" },
  scheduleNotificationAsync: jest.fn(),
  cancelScheduledNotificationAsync: jest.fn(),
}));
jest.mock("../../src/lib/api", () => ({ fetchRadarNowcast: jest.fn() }));
jest.mock("../../src/stores/useWeatherStore", () => ({}));
jest.mock("../../src/lib/storage", () => {
  const values = new Map<string, string>();
  return {
    getString: (key: string, fallback: string) => values.get(key) ?? fallback,
    setString: (key: string, value: string) => values.set(key, value),
    reset: () => values.clear(),
  };
});

const NOW = Date.UTC(2026, 8, 30, 15);
const MIN = 60_000;
const plan: RainAlertPlan = {
  key: "first-run",
  fireAt: NOW + 10 * MIN,
  startAt: NOW + 20 * MIN,
  endAt: null,
  peakInHr: 0.1,
  heavy: false,
  title: "Rain starting soon",
  body: "Rain reaching Boston.",
};

beforeEach(() => {
  jest.clearAllMocks();
  jest.requireMock("../../src/lib/storage").reset();
  jest.mocked(Notifications.scheduleNotificationAsync).mockResolvedValue("notification-id");
});

it("replaces an undelivered alert when a new nowcast predicts the same storm", async () => {
  await syncRainAlert(plan, NOW);
  const refreshed = { ...plan, key: "next-run", fireAt: plan.fireAt + MIN, startAt: plan.startAt + MIN };
  expect(await syncRainAlert(refreshed, NOW + 2 * MIN)).toBe("scheduled");
  expect(Notifications.cancelScheduledNotificationAsync).toHaveBeenCalledWith("notification-id");
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(2);
});

it("reschedules a pending alert when the lead time changes", async () => {
  await syncRainAlert(plan, NOW);
  expect(await syncRainAlert({ ...plan, fireAt: NOW + 5 * MIN }, NOW + MIN)).toBe("scheduled");
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(2);
});

it("keeps an unchanged pending alert", async () => {
  await syncRainAlert(plan, NOW);
  expect(await syncRainAlert(plan, NOW + MIN)).toBe("kept");
  expect(Notifications.cancelScheduledNotificationAsync).not.toHaveBeenCalled();
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(1);
});

it("does not announce the same storm again after its alert has fired", async () => {
  await syncRainAlert(plan, NOW);
  expect(await syncRainAlert({ ...plan, key: "next-run" }, plan.fireAt + MIN)).toBe("skipped");
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(1);
});

it("cancels a pending alert when rain disappears and can schedule it if rain returns", async () => {
  await syncRainAlert(plan, NOW);
  expect(await syncRainAlert(null, NOW + MIN)).toBe("cancelled");
  expect(await syncRainAlert(plan, NOW + 2 * MIN)).toBe("scheduled");
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(2);
});
