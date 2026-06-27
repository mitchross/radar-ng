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

export type NowcastVerdict =
  | { kind: "unavailable" }
  | { kind: "dry" }
  | { kind: "raining"; peakMinute: number; endMinute: number }
  | {
      kind: "starting";
      startMinute: number;
      peakMinute: number;
      endMinute: number;
    };

export function getNowcastVerdict(
  values: number[] | undefined,
): NowcastVerdict {
  if (!values?.length) return { kind: "unavailable" };

  const startMinute = values.findIndex((value) => value > 0.08);
  if (startMinute < 0) return { kind: "dry" };

  const peakMinute = values.reduce(
    (best, value, index) => (value > values[best] ? index : best),
    0,
  );
  let endMinute = values.length - 1;
  while (endMinute > startMinute && values[endMinute] <= 0.05) {
    endMinute -= 1;
  }

  return startMinute === 0
    ? { kind: "raining", peakMinute, endMinute }
    : { kind: "starting", startMinute, peakMinute, endMinute };
}
