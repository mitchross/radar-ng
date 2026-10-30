export type QueryPresentation = {
  kind: "loading" | "error" | "content";
  stale: boolean;
};

export function getForecastScreenState(input: {
  data: unknown;
  isLoading: boolean;
  isError: boolean;
  isFetching: boolean;
}): QueryPresentation {
  if (input.data) return { kind: "content", stale: input.isError };
  if (input.isLoading || input.isFetching) {
    return { kind: "loading", stale: false };
  }
  return { kind: "error", stale: false };
}

export { describeNowcast, getNowcastVerdict } from "../../../web/src/shared/nowcastVerdict";
export type { NowcastVerdict } from "../../../web/src/shared/nowcastVerdict";

export function getAlertsScreenState(input: {
  data: { features: readonly unknown[] } | undefined;
  isLoading: boolean;
  isPending: boolean;
  freshness: "current" | "checking" | "stale" | "offline" | "unavailable";
}): { kind: "loading" | "error" | "empty" | "content" } {
  if (input.data?.features.length) return { kind: "content" };
  if (input.data && input.freshness === "current") return { kind: "empty" };
  if (
    input.freshness === "checking" ||
    (input.freshness === "current" && (input.isLoading || input.isPending))
  ) {
    return { kind: "loading" };
  }
  return { kind: "error" };
}
