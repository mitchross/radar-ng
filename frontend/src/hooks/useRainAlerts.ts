import { useEffect } from "react";
import { useActiveLocation } from "./useActiveLocation";
import { useRadarNowcast } from "./useRadarNowcast";
import { planRainAlert } from "../lib/rainAlerts";
import { cancelPendingRainAlert, syncRainAlert } from "../lib/rainAlertScheduler";
import { setRainAlertsBackgroundRefresh } from "../tasks/rainAlertsTask";
import { useWeatherStore } from "../stores/useWeatherStore";

/**
 * Keeps the "rain starting soon" local notification in step with the radar
 * nowcast while the app is open. Mount once at the root.
 */
export function useRainAlerts(): void {
  const enabled = useWeatherStore((s) => s.rainAlertsEnabled);
  const leadMinutes = useWeatherStore((s) => s.rainAlertLeadMinutes);
  const { name } = useActiveLocation();
  // Only poll for the alert's sake when the feature is on; screens keep their own subscription.
  const { data: nowcast } = useRadarNowcast({ enabled });

  useEffect(() => {
    void setRainAlertsBackgroundRefresh(enabled);
  }, [enabled]);

  useEffect(() => {
    if (!enabled) {
      void cancelPendingRainAlert();
      return;
    }
    if (!nowcast) return;
    void syncRainAlert(planRainAlert(nowcast, name, { leadMinutes }));
  }, [enabled, leadMinutes, nowcast, name]);
}
