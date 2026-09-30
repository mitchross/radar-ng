/**
 * Background refresh for rain alerts. iOS and Android wake the app every so
 * often (15 min at best, usually longer); each wake re-fetches the nowcast for
 * the saved location and re-plans the local notification. The foreground hook
 * does the same every minute while the app is open, so this only covers the
 * gaps. Import this module once at startup: defineTask must run at module
 * scope before any component mounts.
 */
import * as BackgroundTask from "expo-background-task";
import * as TaskManager from "expo-task-manager";
import { installNotificationHandler, refreshRainAlertFromServer } from "../lib/rainAlertScheduler";

export const RAIN_ALERTS_TASK = "radar-ng-rain-alerts";

installNotificationHandler();

try {
  TaskManager.defineTask(RAIN_ALERTS_TASK, async () => {
    try {
      await refreshRainAlertFromServer();
      return BackgroundTask.BackgroundTaskResult.Success;
    } catch {
      return BackgroundTask.BackgroundTaskResult.Failed;
    }
  });
} catch {
  // Not available in this runtime (Expo Go, tests); the foreground path still works.
}

/** Register or drop the periodic wake to match the setting. Never throws. */
export async function setRainAlertsBackgroundRefresh(enabled: boolean): Promise<void> {
  try {
    const registered = await TaskManager.isTaskRegisteredAsync(RAIN_ALERTS_TASK);
    if (enabled && !registered) {
      await BackgroundTask.registerTaskAsync(RAIN_ALERTS_TASK, { minimumInterval: 15 });
    } else if (!enabled && registered) {
      await BackgroundTask.unregisterTaskAsync(RAIN_ALERTS_TASK);
    }
  } catch {
    // Background execution restricted (Low Power Mode, Expo Go): foreground alerts still work.
  }
}
