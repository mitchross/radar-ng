import {
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
});
