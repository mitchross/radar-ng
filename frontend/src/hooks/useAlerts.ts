import { useEffect, useReducer, useSyncExternalStore } from "react";
import { onlineManager, useQuery } from "@tanstack/react-query";
import { fetchAlerts } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";
import {
  ALERT_ALL_CLEAR_MAX_AGE_MS,
  getAlertCollectionSnapshot,
  getAlertFreshnessStatus,
  isAlertDataRecent,
  scheduleAlertTransition,
} from "../lib/alertLifecycle";
import { useAppActive } from "./useAppActive";

const subscribeToOnlineState = (onStoreChange: () => void) =>
  onlineManager.subscribe(() => onStoreChange());
const getOnlineState = () => onlineManager.isOnline();

export function useAlerts() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const appActive = useAppActive();
  const isOnline = useSyncExternalStore(subscribeToOnlineState, getOnlineState, getOnlineState);
  const [, advanceClock] = useReducer((revision: number) => revision + 1, 0);

  const query = useQuery({
    queryKey: ["alerts", latitude, longitude],
    queryFn: ({ signal }) => fetchAlerts(latitude!, longitude!, signal),
    enabled: latitude !== null && longitude !== null,
    refetchInterval: DEFAULTS.ALERTS_REFETCH_MS,
    staleTime: DEFAULTS.ALERTS_REFETCH_MS,
  });

  const now = Date.now();
  const snapshot = query.data
    ? getAlertCollectionSnapshot(query.data, now)
    : undefined;
  const isRecent = query.data !== undefined && isAlertDataRecent(query.dataUpdatedAt, now);
  // Re-render when the all-clear ages out too, so a stalled poll cannot leave "no alerts" on screen.
  const allClearExpiresAt = isRecent ? query.dataUpdatedAt + ALERT_ALL_CLEAR_MAX_AGE_MS : null;
  const alertTransitionAt = snapshot?.nextTransitionAt ?? null;
  const nextTransitionAt = alertTransitionAt === null || allClearExpiresAt === null
    ? alertTransitionAt ?? allClearExpiresAt
    : Math.min(alertTransitionAt, allClearExpiresAt);

  useEffect(() => {
    if (!appActive) return undefined;
    return scheduleAlertTransition(nextTransitionAt, advanceClock);
  }, [appActive, nextTransitionAt]);

  const alertStatus = getAlertFreshnessStatus({
    hasCachedData: query.data !== undefined,
    activeCount: snapshot?.collection.features.length ?? 0,
    invalidCount: snapshot?.invalidCount ?? 0,
    isOnline,
    refreshFailed: query.isError,
    isPending: query.isPending,
    isRecent,
    isFetching: query.isFetching,
  });

  return {
    ...query,
    data: snapshot?.collection,
    alertStatus,
    invalidAlertCount: snapshot?.invalidCount ?? 0,
  };
}
