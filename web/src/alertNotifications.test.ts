import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BrowserRainAlerts } from "./rainAlerts";
import { BrowserAlertNotifications } from "./alertNotifications";
import type { AlertFeature } from "./shared/alertLevels";

const showNotification = vi.fn();
const requestPermission = vi.fn();
const openRadar = vi.fn();
function warning(id: string, event = "Flood Warning"): AlertFeature {
  return { id, properties: { event, severity: "Severe" }, geometry: { type: "Polygon", coordinates: [[[-86, 42], [-85, 43], [-84, 42], [-86, 42]]] } };
}
beforeEach(() => {
  vi.clearAllMocks();
  showNotification.mockResolvedValue(undefined);
  requestPermission.mockResolvedValue("granted");
  vi.stubGlobal("Notification", { permission: "granted", requestPermission });
  vi.stubGlobal("navigator", { serviceWorker: { register: vi.fn().mockResolvedValue({ showNotification }) } });
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value) });
});
afterEach(() => vi.unstubAllGlobals());

it("delivers five severity-ordered notifications with polygon tap data and remembers overflow", async () => {
  const alerts = new BrowserAlertNotifications(new BrowserRainAlerts(), openRadar);
  const batch = [...Array.from({ length: 7 }, (_, i) => warning(String(i))), warning("tor", "Tornado Warning")];
  await alerts.check(batch);
  expect(showNotification).toHaveBeenCalledTimes(5);
  expect(showNotification.mock.calls[0]).toEqual(["CRITICAL — Tornado Warning", expect.objectContaining({ data: { kind: "nws-alert", bounds: [-86, 42, -84, 43] } })]);
  await new BrowserAlertNotifications(new BrowserRainAlerts(), openRadar).check(batch);
  expect(showNotification).toHaveBeenCalledTimes(5);
});
it("records update IDs silently and does not replay delivery failures", async () => {
  const alerts = new BrowserAlertNotifications(new BrowserRainAlerts(), openRadar);
  showNotification.mockRejectedValue(new Error("blocked"));
  await alerts.check([warning("old")]);
  const update = warning("new");
  update.properties.references = [{ identifier: "old" }];
  await alerts.check([update]);
  await alerts.check([warning("new")]);
  expect(showNotification).toHaveBeenCalledOnce();
});
it("test alert asks for permission and is clearly marked, independent of rain opt-in", async () => {
  vi.stubGlobal("Notification", { permission: "default", requestPermission });
  const alerts = new BrowserAlertNotifications(new BrowserRainAlerts(), openRadar);
  expect(await alerts.test()).toBe(true);
  expect(requestPermission).toHaveBeenCalledOnce();
  expect(showNotification).toHaveBeenCalledWith("TEST — Radar NG warning", expect.objectContaining({ body: expect.stringContaining("No weather emergency") }));
});
it("does not claim test delivery when permission or the platform rejects it", async () => {
  const alerts = new BrowserAlertNotifications(new BrowserRainAlerts(), openRadar);
  showNotification.mockRejectedValue(new Error("blocked"));
  expect(await alerts.test()).toBe(false);
  vi.stubGlobal("Notification", { permission: "denied", requestPermission: vi.fn().mockResolvedValue("denied") });
  expect(await alerts.test()).toBe(false);
});
it("persists the threshold without enabling rain alerts", async () => {
  const rain = new BrowserRainAlerts();
  const alerts = new BrowserAlertNotifications(rain, openRadar);
  await alerts.setPreference("watches");
  expect(new BrowserAlertNotifications(rain, openRadar).preference).toBe("watches");
  expect(rain.enabled).toBe(false);
});
