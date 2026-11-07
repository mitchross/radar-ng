import { useCallback, useState } from "react";
import { ensureNotificationPermission } from "../lib/rainAlertScheduler";
import { useWeatherStore } from "../stores/useWeatherStore";

/** Turning the storm notice on asks for notification permission and persists only when granted. */
export function useStormNoticeOptIn() {
  const enabled = useWeatherStore((s) => s.stormNoticeEnabled);
  const setStormNoticeEnabled = useWeatherStore((s) => s.setStormNoticeEnabled);
  const [blocked, setBlocked] = useState(false);

  const setStormNotice = useCallback(
    async (on: boolean) => {
      if (!on) {
        setStormNoticeEnabled(false);
        setBlocked(false);
        return;
      }
      const granted = await ensureNotificationPermission(true);
      setBlocked(!granted);
      setStormNoticeEnabled(granted);
    },
    [setStormNoticeEnabled],
  );

  return { enabled, blocked, setStormNotice };
}
