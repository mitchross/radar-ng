import * as Notifications from "expo-notifications";
import { setIssAlertsEnabled, syncIssAlerts } from "../../src/lib/issAlertScheduler";
import { parseSkyTonight, planIssAlerts } from "../../src/lib/skyTonight";

jest.mock("expo-notifications", () => ({
  AndroidImportance: { DEFAULT: 3 },
  SchedulableTriggerInputTypes: { DATE: "date" },
  scheduleNotificationAsync: jest.fn(),
  cancelScheduledNotificationAsync: jest.fn(),
  setNotificationChannelAsync: jest.fn(),
}));
jest.mock("../../src/lib/storage", () => {
  const values = new Map<string, string>();
  return {
    getString: (key: string, fallback: string) => values.get(key) ?? fallback,
    setString: (key: string, value: string) => values.set(key, value),
    reset: () => values.clear(),
  };
});

const report = parseSkyTonight({
  status: "ok",
  passes: [
    { start: "2026-11-18T22:59:25Z", max_at: "2026-11-18T23:02:17Z", end: "2026-11-18T23:04:53Z", max_elevation_deg: 25.5, start_azimuth_deg: 282, max_azimuth_deg: 341, end_azimuth_deg: 37 },
    { start: "2026-11-17T23:47:42Z", max_at: "2026-11-17T23:50:20Z", end: "2026-11-17T23:50:53Z", max_elevation_deg: 21, start_azimuth_deg: 294, max_azimuth_deg: 345, end_azimuth_deg: 0 },
  ],
});
const clock = { timeZone: "America/Detroit", utcOffsetSeconds: -5 * 3600 };
const NOW = Date.parse("2026-11-17T21:00:00Z");

beforeEach(() => {
  jest.clearAllMocks();
  jest.requireMock("../../src/lib/storage").reset();
  let n = 0;
  jest.mocked(Notifications.scheduleNotificationAsync).mockImplementation(async () => `id-${++n}`);
});

describe("syncIssAlerts", () => {
  it("schedules one notification 10 minutes before each good pass, once", async () => {
    setIssAlertsEnabled(true);
    const plans = planIssAlerts(report, null, NOW, clock);
    expect(await syncIssAlerts(plans)).toEqual({ scheduled: 1, cancelled: 0 });
    const call = jest.mocked(Notifications.scheduleNotificationAsync).mock.calls[0][0];
    expect(call.trigger).toEqual({ type: "date", date: Date.parse("2026-11-18T22:49:25Z"), channelId: "iss-passes" });
    expect(call.content.title).toBe("ISS overhead in 10 minutes");

    expect(await syncIssAlerts(plans)).toEqual({ scheduled: 0, cancelled: 0 });
    expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(1);
  });

  it("cancels what is no longer planned", async () => {
    await syncIssAlerts(planIssAlerts(report, null, NOW, clock));
    expect(await syncIssAlerts([])).toEqual({ scheduled: 0, cancelled: 1 });
    expect(Notifications.cancelScheduledNotificationAsync).toHaveBeenCalledWith("id-1");
  });
});
