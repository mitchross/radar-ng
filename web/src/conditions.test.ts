import { describe, expect, it } from "vitest";
import { nextHourHeadline } from "./nowcast";
import {
  CONDITION_GRADIENTS,
  CONDITION_GRADIENTS_DARK,
  CONDITION_HEADLINES,
  conditionGradient,
  getCumulusCondition,
  rainHeadline,
} from "./shared/conditions";
import { conditionOf, isNightAt } from "./weather";

describe("rainHeadline", () => {
  it("says the fact first, then a nudge", () => {
    expect(rainHeadline(false, 0)).toBe("Rain's here");
    expect(rainHeadline(true, 0)).toBe("Heavy rain, right now");
    expect(rainHeadline(false, 15)).toBe("Rain in 15 min. Heads up");
    expect(rainHeadline(true, 20)).toBe("Heavy rain in 20 min. Heads up");
  });
});

describe("nextHourHeadline", () => {
  it("is null for a dry or drizzly hour", () => {
    expect(nextHourHeadline(Array(60).fill(0))).toBeNull();
    expect(nextHourHeadline(Array(60).fill(0.05))).toBeNull();
  });

  it("names the start minute and heavy rain", () => {
    const starting = Array(60).fill(0).map((_, i) => (i >= 12 ? 0.2 : 0));
    expect(nextHourHeadline(starting)).toBe("Rain in 12 min. Heads up");
    expect(nextHourHeadline(Array(60).fill(0.5))).toBe("Heavy rain, right now");
  });
});

describe("hero headline", () => {
  it("picks the condition by WMO code and day or night", () => {
    expect(CONDITION_HEADLINES[getCumulusCondition(0, false)]).toBe("Sun's out");
    expect(CONDITION_HEADLINES[getCumulusCondition(0, true)]).toBe("Clear and calm");
    expect(CONDITION_HEADLINES[getCumulusCondition(3, false)]).toBe("Clouds, mostly");
    expect(CONDITION_HEADLINES[getCumulusCondition(61, false)]).toBe("Rain. Grab a jacket");
    expect(CONDITION_HEADLINES[getCumulusCondition(81, false)]).toBe("Rain. Grab a jacket");
    expect(CONDITION_HEADLINES[getCumulusCondition(95, false)]).toBe("Thunder's brewing");
    expect(CONDITION_HEADLINES[getCumulusCondition(73, false)]).toBe("Snow's falling");
    expect(CONDITION_HEADLINES[getCumulusCondition(45, false)]).toBe("Fog. Take it slow");
    expect(CONDITION_HEADLINES[getCumulusCondition(null, false)]).toBe("Conditions unavailable");
  });
});

describe("icon and headline agree", () => {
  it("shows a partly cloudy icon under the cloudy headline for WMO code 1", () => {
    expect(CONDITION_HEADLINES[getCumulusCondition(1, false)]).toBe("Clouds, mostly");
    expect(conditionOf(1)).toBe("partly");
    expect(conditionOf(2)).toBe("partly");
  });

  it("derives the icon from the shared WMO mapping", () => {
    expect(conditionOf(0)).toBe("clear");
    expect(conditionOf(3)).toBe("cloudy");
    expect(conditionOf(null)).toBe("cloudy");
    expect(conditionOf(45)).toBe("fog");
    expect(conditionOf(61)).toBe("rain");
    expect(conditionOf(73)).toBe("snow");
    expect(conditionOf(95)).toBe("storm");
  });
});

describe("conditionGradient", () => {
  it("settles every light and dark gradient into its canvas", () => {
    for (const stops of Object.values(CONDITION_GRADIENTS)) expect(stops[4]).toBe("#f6f2ea");
    for (const stops of Object.values(CONDITION_GRADIENTS_DARK)) expect(stops[4]).toBe("#14130f");
    expect(conditionGradient("rain", false)[0]).toBe("#a9c4e6");
    expect(conditionGradient("rain", true)[0]).toBe("#15283f");
  });
});

describe("isNightAt", () => {
  const daily = { time: ["2026-10-11"], sunrise: ["2026-10-11T07:45"], sunset: ["2026-10-11T19:05"] };
  const at = (h: number, m = 0) => Date.UTC(2026, 9, 11, h, m) + 4 * 3600_000; // EDT wall time → instant

  it("uses the location's sunrise and sunset", () => {
    expect(isNightAt(at(7, 30), daily, -14400)).toBe(true);
    expect(isNightAt(at(12), daily, -14400)).toBe(false);
    expect(isNightAt(at(19, 30), daily, -14400)).toBe(true);
  });

  it("falls back to 06:00–20:00 without sun times for that day", () => {
    const none = { time: [], sunrise: [], sunset: [] };
    expect(isNightAt(at(5), none, -14400)).toBe(true);
    expect(isNightAt(at(19), none, -14400)).toBe(false);
  });
});
