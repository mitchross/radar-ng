import { useSyncExternalStore } from "react";
import { formatInMapSummary, inMapStats, inMapStatsVersion, subscribeInMapStats } from "../lib/inMapDiag";

/** The in-map engine's diagnostics as short lines for Settings; repaints as the stats change. */
export function useInMapSummary(): string[] {
  useSyncExternalStore(subscribeInMapStats, inMapStatsVersion);
  return formatInMapSummary(inMapStats);
}
