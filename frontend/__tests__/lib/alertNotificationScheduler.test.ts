import * as Notifications from "expo-notifications";
import { Platform } from "react-native";
import { checkAlertNotifications, sendTestAlert } from "../../src/lib/alertNotificationScheduler";
import type { AlertFeature } from "../../src/lib/alertLevels";

jest.mock("expo-notifications", () => ({
  AndroidImportance: { DEFAULT: 3, HIGH: 4, MAX: 5 },
  setNotificationChannelAsync: jest.fn(),
  scheduleNotificationAsync: jest.fn(),
}));
jest.mock("../../src/lib/api", () => ({ fetchAlerts: jest.fn() }));
jest.mock("../../src/lib/rainAlertScheduler", () => ({ ensureNotificationPermission: jest.fn().mockResolvedValue(true), installNotificationHandler: jest.fn() }));
jest.mock("../../src/stores/useWeatherStore", () => ({ useWeatherStore: { getState: () => ({ alertPreference: "warnings" }) } }));
jest.mock("../../src/lib/storage", () => {
  const values = new Map<string, string>();
  return { getString: (key: string, fallback: string) => values.get(key) ?? fallback, setString: (key: string, value: string) => values.set(key, value), reset: () => values.clear() };
});

function warning(id: string, event = "Flood Warning"): AlertFeature {
  return { id, properties: { event, severity: "Severe" }, geometry: { type: "Polygon", coordinates: [[[-86, 42], [-85, 43], [-84, 42], [-86, 42]]] } };
}
beforeEach(() => {
  jest.clearAllMocks();
  jest.requireMock("../../src/lib/storage").reset();
  jest.requireMock("../../src/lib/rainAlertScheduler").ensureNotificationPermission.mockResolvedValue(true);
  jest.mocked(Notifications.scheduleNotificationAsync).mockResolvedValue("id");
  Object.defineProperty(Platform, "OS", { value: "android", configurable: true });
});

it("uses the critical channel first, caps five and persists overflow across checks", async () => {
  const batch = [...Array.from({ length: 7 }, (_, i) => warning(String(i))), warning("tor", "Tornado Warning")];
  await checkAlertNotifications(batch);
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(5);
  expect(Notifications.scheduleNotificationAsync).toHaveBeenNthCalledWith(1, expect.objectContaining({
    content: expect.objectContaining({ title: "CRITICAL — Tornado Warning", data: { kind: "nws-alert", bounds: [-86, 42, -84, 43] } }), trigger: { channelId: "nws-critical" },
  }));
  await checkAlertNotifications(batch);
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(5);
});
it("silently remembers a reference-linked update", async () => {
  await checkAlertNotifications([warning("old")]);
  const update = warning("new");
  update.properties.references = [{ identifier: "old" }];
  await checkAlertNotifications([update]);
  await checkAlertNotifications([warning("new")]);
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledTimes(1);
});
it("test uses warning sound/channel without touching seen history", async () => {
  expect(await sendTestAlert()).toBe(true);
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledWith(expect.objectContaining({ content: expect.objectContaining({ title: "TEST — Radar NG warning", sound: "default" }), trigger: { channelId: "nws-warning" } }));
});
it("reports denied permission or delivery failure instead of claiming a test was sent", async () => {
  jest.requireMock("../../src/lib/rainAlertScheduler").ensureNotificationPermission.mockResolvedValue(false);
  expect(await sendTestAlert()).toBe(false);
  expect(Notifications.scheduleNotificationAsync).not.toHaveBeenCalled();
  jest.requireMock("../../src/lib/rainAlertScheduler").ensureNotificationPermission.mockResolvedValue(true);
  jest.mocked(Notifications.scheduleNotificationAsync).mockRejectedValue(new Error("unavailable"));
  expect(await sendTestAlert()).toBe(false);
});
it("does not send Android channel triggers on iOS", async () => {
  Object.defineProperty(Platform, "OS", { value: "ios", configurable: true });
  await sendTestAlert();
  expect(Notifications.setNotificationChannelAsync).not.toHaveBeenCalled();
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledWith(expect.objectContaining({ trigger: null }));
});
