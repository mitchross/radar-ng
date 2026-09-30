/**
 * Turns a RainAlertPlan into a local notification on this phone. Nothing here
 * talks to a push service: expo-notifications schedules the OS notification
 * for a future date, and the app cancels or replaces it as the nowcast changes.
 */
import * as Notifications from "expo-notifications";
import { AndroidImportance } from "expo-notifications";
import { Platform } from "react-native";
import { fetchRadarNowcast } from "./api";
import { activeLocationName } from "./locationLabel";
import { isFallbackLocation } from "./locationStatus";
import {
  parseLastRainAlert,
  planRainAlert,
  shouldNotify,
  type LastRainAlert,
  type RainAlertPlan,
} from "./rainAlerts";
import { getString, setString } from "./storage";
import { DEFAULT_PLACE, useWeatherStore } from "../stores/useWeatherStore";

export const RAIN_CHANNEL_ID = "rain-alerts";
const SCHEDULED_KEY = "rainAlertScheduled";
const LAST_KEY = "rainAlertLast";

interface Scheduled {
  id: string;
  key: string;
  fireAt: number;
}

export type RainAlertSync = "scheduled" | "kept" | "cancelled" | "skipped";

let handlerInstalled = false;

/** Show rain alerts even while the app is in the foreground. */
export function installNotificationHandler(): void {
  if (handlerInstalled) return;
  handlerInstalled = true;
  try {
    Notifications.setNotificationHandler({
      handleNotification: async () => ({
        shouldShowBanner: true,
        shouldShowList: true,
        shouldPlaySound: true,
        shouldSetBadge: false,
      }),
    });
  } catch {
    // Expo Go or a platform without the native module: alerts are simply off.
  }
}

/** True when notifications may be shown; prompts only when `prompt` is set and the OS still allows asking. */
export async function ensureNotificationPermission(prompt: boolean): Promise<boolean> {
  try {
    const current = await Notifications.getPermissionsAsync();
    if (current.granted) return true;
    if (!prompt || current.canAskAgain === false) return false;
    const next = await Notifications.requestPermissionsAsync({
      ios: { allowAlert: true, allowSound: true, allowBadge: false },
    });
    return next.granted;
  } catch {
    return false;
  }
}

async function ensureChannel(): Promise<void> {
  if (Platform.OS !== "android") return;
  await Notifications.setNotificationChannelAsync(RAIN_CHANNEL_ID, {
    name: "Rain alerts",
    description: "Rain reaching your location soon, from your own radar server.",
    importance: AndroidImportance.HIGH,
  });
}

function readScheduled(): Scheduled | null {
  try {
    const parsed = JSON.parse(getString(SCHEDULED_KEY, "")) as Partial<Scheduled>;
    if (typeof parsed.id === "string" && typeof parsed.key === "string" && typeof parsed.fireAt === "number") {
      return { id: parsed.id, key: parsed.key, fireAt: parsed.fireAt };
    }
  } catch {}
  return null;
}

function writeScheduled(value: Scheduled | null): void {
  setString(SCHEDULED_KEY, value ? JSON.stringify(value) : "");
}

function readLast(): LastRainAlert | null {
  return parseLastRainAlert(getString(LAST_KEY, ""));
}

/** Drop a pending notification; an alert that never showed is forgotten so the storm can be announced again. */
export async function cancelPendingRainAlert(now = Date.now()): Promise<void> {
  const pending = readScheduled();
  if (!pending) return;
  try {
    await Notifications.cancelScheduledNotificationAsync(pending.id);
  } catch {}
  writeScheduled(null);
  const last = readLast();
  if (last && last.key === pending.key && pending.fireAt > now) setString(LAST_KEY, "");
}

/**
 * Make the OS schedule match the plan: keep an identical pending alert,
 * replace a changed one, cancel when the rain is gone, and never announce the
 * same event twice.
 */
export async function syncRainAlert(plan: RainAlertPlan | null, now = Date.now()): Promise<RainAlertSync> {
  const pending = readScheduled();
  if (!plan) {
    if (pending) await cancelPendingRainAlert(now);
    return "cancelled";
  }
  if (pending && pending.key === plan.key && Math.abs(pending.fireAt - plan.fireAt) <= 60_000) return "kept";
  // A future notification has not announced anything yet. Cancel it before
  // deduplication so a new run or lead-time preference can replace the plan.
  if (pending) await cancelPendingRainAlert(now);
  if (!shouldNotify(plan, readLast(), now)) {
    return "skipped";
  }
  await ensureChannel();
  const immediate = plan.fireAt <= now + 2_000;
  const id = await Notifications.scheduleNotificationAsync({
    content: {
      title: plan.title,
      body: plan.body,
      sound: "default",
      data: { key: plan.key, startAt: plan.startAt },
    },
    trigger: immediate
      ? Platform.OS === "android"
        ? { channelId: RAIN_CHANNEL_ID }
        : null
      : { type: Notifications.SchedulableTriggerInputTypes.DATE, date: plan.fireAt, channelId: RAIN_CHANNEL_ID },
  });
  writeScheduled({ id, key: plan.key, fireAt: plan.fireAt });
  const last: LastRainAlert = { key: plan.key, startAt: plan.startAt, firedAt: plan.fireAt };
  setString(LAST_KEY, JSON.stringify(last));
  return "scheduled";
}

/**
 * One full pass from the store: fetch the nowcast for the active location and
 * sync the notification. Used by the background task, where no screen is
 * mounted; the foreground hook reuses the screens' query instead.
 */
export async function refreshRainAlertFromServer(): Promise<RainAlertSync | "disabled" | "no-location"> {
  const s = useWeatherStore.getState();
  if (!s.rainAlertsEnabled) {
    await cancelPendingRainAlert();
    return "disabled";
  }
  if (s.latitude == null || s.longitude == null) return "no-location";
  const nowcast = await fetchRadarNowcast(s.serverUrl, s.latitude, s.longitude);
  const fallback = isFallbackLocation(s.locationMode, s.locationStatus) ? DEFAULT_PLACE : null;
  const place = activeLocationName(s.locationMode, s.selectedPlace, s.devicePlace, fallback);
  return syncRainAlert(planRainAlert(nowcast, place, { leadMinutes: s.rainAlertLeadMinutes }));
}
