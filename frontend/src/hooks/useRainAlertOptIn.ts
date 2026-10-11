import { useCallback, useState } from "react";
import { ensureNotificationPermission } from "../lib/rainAlertScheduler";
import { useWeatherStore } from "../stores/useWeatherStore";

/**
 * The one way to turn rain alerts on: ask for notification permission, then
 * persist the preference only when it was granted. Settings, the Home rain
 * banner and the Nowcast hero all go through here.
 */
export function useRainAlertOptIn() {
  const enabled = useWeatherStore((s) => s.rainAlertsEnabled);
  const setRainAlertsEnabled = useWeatherStore((s) => s.setRainAlertsEnabled);
  const [blocked, setBlocked] = useState(false);
  const [busy, setBusy] = useState(false);

  const setRainAlerts = useCallback(
    async (on: boolean) => {
      if (!on) {
        setRainAlertsEnabled(false);
        setBlocked(false);
        return;
      }
      setBusy(true);
      try {
        const granted = await ensureNotificationPermission(true);
        setBlocked(!granted);
        setRainAlertsEnabled(granted);
      } finally {
        setBusy(false);
      }
    },
    [setRainAlertsEnabled],
  );

  return { enabled, blocked, busy, setRainAlerts };
}
