/**
 * "Strong storm may reach you in ~N min", as a local notification on this
 * phone. Same on-device model as the rain alerts: the app already holds the
 * answer (tracked cells from its own server), so it posts the notification
 * itself. No push service, and the location never leaves the phone: only the
 * server-wide /api/storms list is fetched.
 */
import * as Notifications from "expo-notifications";
import { AndroidImportance } from "expo-notifications";
import { Platform } from "react-native";
import { fetchWithTimeout } from "./api";
import { getString, setString } from "./storage";
import { parseStormCells, type StormCellInfo } from "./stormTracks";
import { useWeatherStore } from "../stores/useWeatherStore";
import {
  approachCopy,
  parseSeenApproaches,
  stormApproaches,
  takeNewApproaches,
  type StormApproach,
} from "../../../web/src/shared/stormCone";

export const STORM_CHANNEL_ID = "storm-notice";
const SEEN_KEY = "stormNoticeSeen";

async function ensureChannel(): Promise<void> {
  if (Platform.OS !== "android") return;
  await Notifications.setNotificationChannelAsync(STORM_CHANNEL_ID, {
    name: "Strong storm nearby",
    description: "A strong storm cell on course for your location, estimated from your own radar server.",
    importance: AndroidImportance.HIGH,
  });
}

/**
 * Approaching cells for the place; any not yet announced post one notification.
 * Cells are remembered by id, so a later poll of the same storm stays quiet.
 */
export async function checkStormNotice(
  cells: StormCellInfo[],
  place: [number, number],
  now = Date.now(),
): Promise<StormApproach[]> {
  const approaches = stormApproaches(cells, place);
  const { fresh, seen } = takeNewApproaches(approaches, parseSeenApproaches(getString(SEEN_KEY, "")), now);
  if (fresh.length === 0) return approaches;
  // Remember first: a failed post should not turn into a repeat on every poll.
  setString(SEEN_KEY, JSON.stringify(seen));
  try {
    await ensureChannel();
    const { title, body } = approachCopy(fresh[0]);
    await Notifications.scheduleNotificationAsync({
      content: { title, body, sound: "default", data: { kind: "storm-approach", cellId: fresh[0].id } },
      trigger: Platform.OS === "android" ? { channelId: STORM_CHANNEL_ID } : null,
    });
  } catch {
    // Notifications unavailable (Expo Go, permission revoked): the Home card still shows it.
  }
  return approaches;
}

/** Background wake: fetch the storm list and check the saved location. */
export async function refreshStormNoticeFromServer(): Promise<"disabled" | "no-location" | "checked"> {
  const s = useWeatherStore.getState();
  if (!s.stormNoticeEnabled) return "disabled";
  if (s.latitude == null || s.longitude == null) return "no-location";
  const r = await fetchWithTimeout(`${s.serverUrl}/api/storms`);
  if (!r.ok) throw new Error(`storms fetch ${r.status}`);
  await checkStormNotice(parseStormCells(await r.json()), [s.longitude, s.latitude]);
  return "checked";
}
