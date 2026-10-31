import { useSyncExternalStore } from "react";
import { formatInMapSummary, inMapStats, subscribeInMapStats } from "../lib/inMapDiag";

const summaryText = () => formatInMapSummary(inMapStats).join("\n");

/**
 * The in-map engine's diagnostics as short lines for Settings. The snapshot is the text itself, read
 * from the live stats on every render and on every throttled notify, so a screen that mounts after
 * the engine did its work shows the current values and one that stays mounted repaints as they change.
 */
export function useInMapSummary(): string[] {
  return useSyncExternalStore(subscribeInMapStats, summaryText).split("\n");
}
