/**
 * Condition gradients and voice. Mirrors frontend/src/lib/cumulusTheme.ts
 * (and rainHeadline from frontend/src/lib/forecastView.ts); change both together.
 */
export type CumulusCondition = "clearDay" | "clearNight" | "cloudy" | "rain" | "storm" | "snow" | "fog" | "unknown";

export type ConditionGradient = readonly [string, string, string, string, string];

/** 5-stop gradient per condition: a bold top that settles into the paper canvas. */
export const CONDITION_GRADIENTS: Record<CumulusCondition, ConditionGradient> = {
  clearDay:   ["#ffd27a", "#ffe3a6", "#f9ecd0", "#f7f2e8", "#f6f2ea"],
  clearNight: ["#b9c3ea", "#d2d8f0", "#e7e6ee", "#f3f0ea", "#f6f2ea"],
  cloudy:     ["#cfd5dc", "#dfe2e5", "#ebeae8", "#f3f0ea", "#f6f2ea"],
  rain:       ["#a9c4e6", "#c6d7ec", "#dfe5ec", "#f0efea", "#f6f2ea"],
  storm:      ["#aaa3d0", "#c3bedc", "#dcd9e4", "#eeece9", "#f6f2ea"],
  snow:       ["#d4e4f5", "#e3ecf5", "#eeeff1", "#f4f1ec", "#f6f2ea"],
  fog:        ["#d3d0ca", "#e0ddd6", "#ebe8e0", "#f3f0e8", "#f6f2ea"],
  unknown:    ["#ebe5d8", "#f0ebe0", "#f3efe5", "#f5f1e9", "#f6f2ea"],
};

/** Dark counterparts: the same hue family, settling into the dark canvas. */
export const CONDITION_GRADIENTS_DARK: Record<CumulusCondition, ConditionGradient> = {
  clearDay:   ["#3d2c10", "#2d2314", "#1f1b12", "#171510", "#14130f"],
  clearNight: ["#171c3a", "#171a2c", "#15161f", "#141414", "#14130f"],
  cloudy:     ["#262a30", "#202327", "#1a1b1c", "#161614", "#14130f"],
  rain:       ["#15283f", "#162130", "#151a21", "#141615", "#14130f"],
  storm:      ["#2a2045", "#231c35", "#1b1724", "#161416", "#14130f"],
  snow:       ["#222a34", "#1d2229", "#181a1e", "#151615", "#14130f"],
  fog:        ["#262520", "#20201b", "#1a1916", "#161511", "#14130f"],
  unknown:    ["#262219", "#1f1c15", "#191712", "#161410", "#14130f"],
};

export function conditionGradient(condition: CumulusCondition, dark: boolean): ConditionGradient {
  return (dark ? CONDITION_GRADIENTS_DARK : CONDITION_GRADIENTS)[condition];
}

/** The hero's one-line take on the sky. */
export const CONDITION_HEADLINES: Record<CumulusCondition, string> = {
  clearDay: "Sun's out",
  clearNight: "Clear and calm",
  cloudy: "Clouds, mostly",
  rain: "Rain. Grab a jacket",
  storm: "Thunder's brewing",
  snow: "Snow's falling",
  fog: "Fog. Take it slow",
  unknown: "Conditions unavailable",
};

/** WMO weather code + day/night → condition. A missing code is "unknown", not clear. */
export function getCumulusCondition(weatherCode: number | null | undefined, isNight: boolean): CumulusCondition {
  if (weatherCode == null) return "unknown";
  if (weatherCode === 0) return isNight ? "clearNight" : "clearDay";
  if (weatherCode <= 3) return "cloudy";
  if (weatherCode <= 48) return "fog";
  if (weatherCode <= 67) return "rain";
  if (weatherCode <= 77 || (weatherCode >= 85 && weatherCode <= 86)) return "snow";
  if (weatherCode >= 95) return "storm";
  if (weatherCode >= 80) return "rain";
  return "cloudy";
}

/** The next-hour rain headline: the fact first, then a nudge. */
export function rainHeadline(heavy: boolean, startMinutes: number): string {
  if (startMinutes <= 0) return heavy ? "Heavy rain, right now" : "Rain's here";
  return heavy ? `Heavy rain in ${startMinutes} min. Heads up` : `Rain in ${startMinutes} min. Heads up`;
}
