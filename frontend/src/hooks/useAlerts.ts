import { useEffect, useReducer, useSyncExternalStore } from "react";
import { onlineManager, useQuery } from "@tanstack/react-query";
import { fetchAlerts } from "../lib/api";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";
import {
  getAlertCollectionSnapshot,
  getAlertFreshnessStatus,
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

  const snapshot = query.data
    ? getAlertCollectionSnapshot(query.data, Date.now())
    : undefined;

  useEffect(() => {
    if (!appActive) return undefined;
    return scheduleAlertTransition(snapshot?.nextTransitionAt ?? null, advanceClock);
  }, [appActive, snapshot?.nextTransitionAt]);

  const alertStatus = getAlertFreshnessStatus({
    hasCachedData: query.data !== undefined,
    activeCount: snapshot?.collection.features.length ?? 0,
    invalidCount: snapshot?.invalidCount ?? 0,
    isOnline,
    refreshFailed: query.isError,
    isPending: query.isPending,
  });

  return {
    ...query,
    data: snapshot?.collection,
    alertStatus,
    invalidAlertCount: snapshot?.invalidCount ?? 0,
  };
}
