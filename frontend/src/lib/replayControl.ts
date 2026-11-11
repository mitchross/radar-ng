import type { QueryClient } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";

/** Leave replay: clear its state, drop its cached data, and force a fresh live manifest fetch. */
export async function exitReplay(queryClient: Pick<QueryClient, "invalidateQueries" | "removeQueries">): Promise<void> {
  const { serverUrl, returnToLive } = useWeatherStore.getState();
  returnToLive();
  queryClient.removeQueries({ queryKey: ["replay-manifest"] });
  queryClient.removeQueries({ queryKey: ["replay-warnings"] });
  // The disk-seeded live copy is never shown: frames wait for a fetch newer than the return.
  await queryClient.invalidateQueries({ queryKey: ["manifest", serverUrl], refetchType: "all" });
}
