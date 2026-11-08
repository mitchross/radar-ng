import { useEffect } from "react";
import { useRouter, useRootNavigationState } from "expo-router";
import * as Notifications from "expo-notifications";
import { useAlerts } from "./useAlerts";
import { checkAlertNotifications } from "../lib/alertNotificationScheduler";
import { parseAlertBounds } from "../lib/alertLevels";
import { useWeatherStore } from "../stores/useWeatherStore";

/** One root owner for NWS interruptions and warm/cold notification taps. */
export function useAlertNotifications(): void {
  const { data, dataUpdatedAt, alertStatus } = useAlerts();
  const router = useRouter();
  const navigation = useRootNavigationState();

  useEffect(() => {
    // Never announce restored, stale data after an offline cold start.
    if (data && alertStatus.kind === "current") void checkAlertNotifications(data.features);
  }, [data, dataUpdatedAt, alertStatus]);

  useEffect(() => {
    if (!navigation?.key) return;
    let handled: string | undefined;
    const open = (response: Notifications.NotificationResponse | null) => {
      if (!response) return;
      const request = response.notification.request;
      if (request.content.data?.kind !== "nws-alert" || request.identifier === handled) return;
      handled = request.identifier;
      const bounds = parseAlertBounds(request.content.data.bounds);
      if (bounds) useWeatherStore.getState().setFocusBounds(bounds);
      router.push("/radar");
      void Notifications.clearLastNotificationResponseAsync().catch(() => {});
    };
    try {
      const subscription = Notifications.addNotificationResponseReceivedListener(open);
      void Notifications.getLastNotificationResponseAsync().then(open).catch(() => {});
      return () => subscription.remove();
    } catch { /* Expo Go or no native notifications module */ }
  }, [navigation?.key, router]);
}
