import { onlineManager } from "@tanstack/react-query";

interface AppStateSubscription {
  remove: () => void;
}

interface AppStateSource {
  currentState: string | null;
  addEventListener: (
    event: "change",
    listener: (state: string) => void,
  ) => AppStateSubscription;
}

interface ConnectivitySource {
  addEventListener: (
    listener: (state: { isConnected: boolean | null }) => void,
  ) => () => void;
}

/** Keep TanStack Query focus aligned with native foreground state. */
export function bindAppFocus(
  appState: AppStateSource,
  setFocused: (focused: boolean) => void,
): () => void {
  setFocused(appState.currentState === "active");
  const subscription = appState.addEventListener("change", (state) => {
    setFocused(state === "active");
  });
  return () => subscription.remove();
}

/**
 * A connected LAN is enough for a self-hosted Radar server. Do not gate this
 * on isInternetReachable, which can be false while the local server works.
 */
export function bindNetworkOnline(
  connectivity: ConnectivitySource,
  setOnline: (online: boolean) => void,
): () => void {
  return connectivity.addEventListener((state) => {
    setOnline(state.isConnected !== false);
  });
}

/** Skip awaited manual refetches while Query is paused offline. */
export async function runOnlineRefresh(
  refresh: () => Promise<unknown>,
  isOnline: () => boolean = () => onlineManager.isOnline(),
): Promise<boolean> {
  if (!isOnline()) return false;
  await refresh();
  return true;
}
