/**
 * ISS pass notifications, about 10 minutes before a good pass (25°+ high, not
 * cloudy). Same on-device model as the rain notice: the app already has the
 * passes from its own server and schedules the OS notifications itself.
 */
import * as Notifications from "expo-notifications";
import { AndroidImportance } from "expo-notifications";
import { Platform } from "react-native";
import { getString, setString } from "./storage";
import type { IssAlertPlan } from "./skyTonight";

export const ISS_CHANNEL_ID = "iss-passes";
export const ISS_ALERTS_KEY = "issAlertsEnabled";
const SCHEDULED_KEY = "issAlertScheduled";

interface Scheduled {
  id: string;
  key: string;
}

export const issAlertsEnabled = () => getString(ISS_ALERTS_KEY, "0") === "1";
export const setIssAlertsEnabled = (on: boolean) => setString(ISS_ALERTS_KEY, on ? "1" : "0");

function readScheduled(): Scheduled[] {
  try {
    const v = JSON.parse(getString(SCHEDULED_KEY, "[]"));
    return Array.isArray(v) ? v.filter((s) => typeof s?.id === "string" && typeof s?.key === "string") : [];
  } catch {
    return [];
  }
}

async function ensureChannel(): Promise<void> {
  if (Platform.OS !== "android") return;
  await Notifications.setNotificationChannelAsync(ISS_CHANNEL_ID, {
    name: "ISS passes",
    description: "The space station is about to pass overhead in a clear sky.",
    importance: AndroidImportance.DEFAULT,
  });
}

/** Make the OS schedule match the plans: keep ones already scheduled, cancel the rest, add new ones. */
export async function syncIssAlerts(plans: IssAlertPlan[]): Promise<{ scheduled: number; cancelled: number }> {
  const want = new Map(plans.map((p) => [p.key, p]));
  const pending = readScheduled();
  const keep: Scheduled[] = [];
  let cancelled = 0;
  for (const s of pending) {
    if (want.has(s.key)) {
      keep.push(s);
      want.delete(s.key);
      continue;
    }
    cancelled++;
    try {
      await Notifications.cancelScheduledNotificationAsync(s.id);
    } catch {}
  }
  let scheduled = 0;
  if (want.size) {
    try {
      await ensureChannel();
      for (const p of want.values()) {
        const id = await Notifications.scheduleNotificationAsync({
          content: { title: p.title, body: p.body, sound: "default", data: { kind: "iss-pass", key: p.key, startAt: p.startAt } },
          trigger: { type: Notifications.SchedulableTriggerInputTypes.DATE, date: p.fireAt, channelId: ISS_CHANNEL_ID },
        });
        keep.push({ id, key: p.key });
        scheduled++;
      }
    } catch {
      // Notifications unavailable (Expo Go, permission revoked): the card still lists the passes.
    }
  }
  setString(SCHEDULED_KEY, JSON.stringify(keep));
  return { scheduled, cancelled };
}
