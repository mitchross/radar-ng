import { act, renderHook } from "@testing-library/react-native";
import * as Notifications from "expo-notifications";
import { useAlertNotifications } from "../../src/hooks/useAlertNotifications";

jest.mock("expo-router", () => {
  const router = { push: jest.fn() };
  return { useRouter: () => router, useRootNavigationState: () => ({ key: "root" }) };
});
jest.mock("../../src/hooks/useAlerts", () => ({ useAlerts: () => ({ data: undefined, dataUpdatedAt: 0, alertStatus: { kind: "current" } }) }));
jest.mock("../../src/lib/alertNotificationScheduler", () => ({ checkAlertNotifications: jest.fn() }));
jest.mock("../../src/stores/useWeatherStore", () => {
  const state = { setFocusBounds: jest.fn() };
  return { useWeatherStore: { getState: () => state } };
});
jest.mock("expo-notifications", () => ({
  addNotificationResponseReceivedListener: jest.fn(),
  getLastNotificationResponseAsync: jest.fn(),
  clearLastNotificationResponseAsync: jest.fn(),
}));

function response(bounds: unknown = [-86, 42, -84, 43]): Notifications.NotificationResponse {
  return { notification: { request: { identifier: "nws-1", content: { data: { kind: "nws-alert", bounds } } } } } as unknown as Notifications.NotificationResponse;
}
const push = () => jest.requireMock("expo-router").useRouter().push;
const focus = () => jest.requireMock("../../src/stores/useWeatherStore").useWeatherStore.getState().setFocusBounds;
const remove = jest.fn();
beforeEach(() => {
  jest.clearAllMocks();
  jest.mocked(Notifications.getLastNotificationResponseAsync).mockResolvedValue(null);
  jest.mocked(Notifications.addNotificationResponseReceivedListener).mockReturnValue({ remove });
  jest.mocked(Notifications.clearLastNotificationResponseAsync).mockResolvedValue(undefined);
});

it("opens the radar with the polygon after a cold-start notification tap", async () => {
  jest.mocked(Notifications.getLastNotificationResponseAsync).mockResolvedValue(response());
  await renderHook(() => useAlertNotifications());
  expect(focus()).toHaveBeenCalledWith([-86, 42, -84, 43]);
  expect(push()).toHaveBeenCalledWith("/radar");
  expect(Notifications.clearLastNotificationResponseAsync).toHaveBeenCalled();
});

it("handles a warm tap only once and removes its listener on unmount", async () => {
  const { unmount } = await renderHook(() => useAlertNotifications());
  const listener = jest.mocked(Notifications.addNotificationResponseReceivedListener).mock.calls[0][0];
  await act(async () => { listener(response()); listener(response()); });
  expect(push()).toHaveBeenCalledTimes(1);
  await unmount();
  expect(remove).toHaveBeenCalled();
});

it("opens the radar without trying to frame missing or invalid polygons", async () => {
  jest.mocked(Notifications.getLastNotificationResponseAsync).mockResolvedValue(response([0, 0, Infinity, 3]));
  await renderHook(() => useAlertNotifications());
  expect(push()).toHaveBeenCalledWith("/radar");
  expect(focus()).not.toHaveBeenCalled();
});
