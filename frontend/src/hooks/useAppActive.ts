import { useEffect, useState } from "react";
import { AppState, type AppStateStatus } from "react-native";

// Android reports "unknown"/null briefly at launch — only background/inactive count as away.
const isActive = (s: AppStateStatus | null) => s !== "background" && s !== "inactive";

/** True while the app is foregrounded; drives pausing of timers that AppState-unaware libraries keep running. */
export function useAppActive(): boolean {
  const [active, setActive] = useState(() => isActive(AppState.currentState));
  useEffect(() => {
    const sub = AppState.addEventListener("change", (s) => setActive(isActive(s)));
    return () => sub.remove();
  }, []);
  return active;
}
