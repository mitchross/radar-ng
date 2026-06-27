import {
  describeNowcast,
  getAlertsScreenState,
  getForecastScreenState,
  getNowcastVerdict,
} from "../../src/lib/weatherPresentation";

describe("forecast presentation", () => {
  it("does not label unavailable forecast data as loading after a request fails", () => {
    expect(getForecastScreenState({
      data: undefined,
      isLoading: false,
      isError: true,
      isFetching: false,
    })).toEqual({ kind: "error", stale: false });
  });

  it("keeps stale forecast data visible while refresh fails", () => {
    expect(getForecastScreenState({
      data: { current: {} },
      isLoading: false,
      isError: true,
      isFetching: false,
    })).toEqual({ kind: "content", stale: true });
  });

  it("distinguishes dry nowcast from unavailable minute data", () => {
    expect(getNowcastVerdict(undefined)).toEqual({ kind: "unavailable" });
    expect(getNowcastVerdict([0, 0, 0, 0])).toEqual({ kind: "dry" });
  });

  it("summarizes precipitation timing without relying on the chart", () => {
    expect(describeNowcast({
      kind: "starting",
      startMinute: 15,
      peakMinute: 30,
      endMinute: 50,
    })).toBe(
      "Rain starts in 15 minutes, peaks at 30 minutes, and ends near 50 minutes.",
    );
  });

  it.each([
    [{ data: undefined, isLoading: true, isError: false }, "loading"],
    [{ data: undefined, isLoading: false, isError: true }, "error"],
    [{ data: { features: [] }, isLoading: false, isError: false }, "empty"],
    [{ data: { features: [{ id: "one" }] }, isLoading: false, isError: false }, "content"],
  ] as const)("maps alert query state to %s", (input, kind) => {
    expect(getAlertsScreenState(input).kind).toBe(kind);
  });
});
