import type { Query } from "@tanstack/react-query";

/**
 * Query families saved to disk so a cold start renders the last known data
 * immediately and refreshes it in the background. Alerts are included: their
 * freshness rules (src/lib/alertLifecycle.ts) key off the original fetch
 * time, so a restored empty list never reads as a current all-clear.
 */
export const PERSISTED_QUERY_FAMILIES = ["forecast", "alerts", "radar-nowcast"] as const;

/** Restored data older than this is dropped rather than shown. */
export const PERSIST_MAX_AGE_MS = 24 * 60 * 60_000;

export function shouldPersistQuery(query: Pick<Query, "queryKey" | "state">): boolean {
  const family = query.queryKey[0];
  return (
    typeof family === "string" &&
    (PERSISTED_QUERY_FAMILIES as readonly string[]).includes(family) &&
    query.state.status === "success"
  );
}
