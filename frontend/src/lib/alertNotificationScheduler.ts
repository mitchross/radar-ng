import * as Notifications from "expo-notifications";
import { AndroidImportance } from "expo-notifications";
import { Platform } from "react-native";
import { fetchAlerts } from "./api";
import { ensureNotificationPermission, installNotificationHandler } from "./rainAlertScheduler";
import { getString, setString } from "./storage";
import { useWeatherStore } from "../stores/useWeatherStore";
import { alertLevel, parseAlertSeen, polygonAlertBounds, selectAlertNotifications, TEST_ALERT, type AlertFeature, type AlertLevel } from "./alertLevels";

const SEEN_KEY = "nwsAlertsSeen";
const CHANNELS = {
  chime: { importance: AndroidImportance.DEFAULT, vibrationPattern: [0, 150] },
  warning: { importance: AndroidImportance.HIGH, vibrationPattern: [0, 250, 150, 250] },
  critical: { importance: AndroidImportance.MAX, vibrationPattern: [0, 500, 200, 500, 200, 500] },
};

async function ensureChannel(level: AlertLevel): Promise<void> {
  if (Platform.OS === "android") await Notifications.setNotificationChannelAsync(`nws-${level}`, {
    name: `NWS ${level}`, sound: "default", ...CHANNELS[level],
  });
}

export async function requestAlertNotificationPermission(): Promise<boolean> {
  try {
    // Android needs a channel before its first notification permission prompt.
    for (const level of ["chime", "warning", "critical"] as const) await ensureChannel(level);
    return await ensureNotificationPermission(true);
  } catch { return false; }
}

async function deliver(level: AlertLevel, title: string, body: string, bounds: ReturnType<typeof polygonAlertBounds>): Promise<void> {
  installNotificationHandler();
  const channelId = `nws-${level}`;
  await ensureChannel(level);
  await Notifications.scheduleNotificationAsync({
    content: {
      title, body, sound: "default", data: { kind: "nws-alert", bounds },
      interruptionLevel: level === "chime" ? "active" : "timeSensitive",
    },
    trigger: Platform.OS === "android" ? { channelId } : null,
  });
}

export async function sendTestAlert(): Promise<boolean> {
  if (!await requestAlertNotificationPermission()) return false;
  try {
    await deliver("warning", TEST_ALERT.title, TEST_ALERT.body, null);
    return true;
  } catch { return false; }
}

/** Record synchronously before delivery awaits: concurrent checks cannot announce twice. */
export async function checkAlertNotifications(alerts: readonly AlertFeature[], now = Date.now()): Promise<void> {
  if (!await ensureNotificationPermission(false)) return;
  const { fresh, seen } = selectAlertNotifications(alerts, parseAlertSeen(getString(SEEN_KEY, "")), useWeatherStore.getState().alertPreference, now);
  setString(SEEN_KEY, JSON.stringify(seen));
  for (const alert of fresh) {
    const level = alertLevel(alert.properties);
    try {
      await deliver(level, `${level.toUpperCase()} — ${alert.properties.event}`, alert.properties.headline ?? alert.properties.event, polygonAlertBounds(alert.geometry));
    } catch { /* keep history: a failed post must not turn into repeated interruptions */ }
  }
}

export async function refreshAlertNotificationsFromServer(): Promise<void> {
  const s = useWeatherStore.getState();
  if (s.latitude == null || s.longitude == null || !await ensureNotificationPermission(false)) return;
  const alerts = await fetchAlerts(s.serverUrl, s.latitude, s.longitude);
  await checkAlertNotifications(alerts.features);
}
