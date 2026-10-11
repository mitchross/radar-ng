import { Platform } from "react-native";

export const cumulus = {
  background: "#f6f2ea",
  accent: "#c2603a",
  accentBright: "#e3794f",
  accentDim: "#914424",
  accentSoft: "rgba(194, 96, 58, 0.12)",
  accentBorder: "rgba(194, 96, 58, 0.30)",

  ink: "#211f1b",
  inkDim: "#5d574d",
  inkMuted: "#8c857a",
  inkFaint: "#a39a8a",
  inkLine: "#e7e0d3",

  card: "#ffffff",
  cardStrong: "#fbf9f5",
  cardLine: "#eee6d8",

  rain: "#4d7fb8",
  rainHeavy: "#3f6fd6",
  snow: "#b5cde6",
  sun: "#f0c34e",
  temp: "#df6a3c",
  cold: "#6db4d8",
  hot: "#df6a3c",
  alert: "#df6a6a",
  ok: "#2e9e63",
} as const;

export const cumulusFonts = {
  display: Platform.OS === "ios" ? "Georgia" : "serif",
  ui: Platform.OS === "ios" ? "System" : "sans-serif",
  mono: Platform.OS === "ios" ? "Courier" : "monospace",
} as const;

export type CumulusCondition =
  | "clearDay"
  | "clearNight"
  | "cloudy"
  | "rain"
  | "storm"
  | "snow"
  | "fog"
  | "unknown";

export type ConditionGradient = readonly [string, string, string, string, string];

/**
 * 5-stop gradient per condition: a bold, condition-coloured top that settles
 * into the paper canvas so cards and ink text keep their contrast.
 */
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

/** Dark-mode counterparts: the same hue family, settling into the dark canvas. */
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

/** The Home hero's one-line take on the sky. Short, a little voice, never cute at the data's expense. */
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

/**
 * Map WMO weather code + day/night to Cumulus condition. A missing code is
 * "unknown": null would otherwise compare as 0 and read as clear or cloudy.
 */
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

/** Map WMO weather code to Cumulus WeatherIcon kind. */
export type IconKind =
  | "sun"
  | "moon"
  | "partlyCloudy"
  | "cloudy"
  | "overcast"
  | "rain"
  | "heavyRain"
  | "storm"
  | "snow"
  | "fog"
  | "hail"
  | "unknown";

export function getIconKind(weatherCode: number | null | undefined, isNight: boolean): IconKind {
  if (weatherCode == null) return "unknown";
  // Thunderstorm with hail must be checked before the generic >= 95 storm case.
  if (weatherCode === 96 || weatherCode === 99) return "hail";
  if (weatherCode === 0) return isNight ? "moon" : "sun";
  if (weatherCode <= 2) return "partlyCloudy";
  if (weatherCode === 3) return "overcast";
  if (weatherCode <= 48) return "fog";
  if (weatherCode <= 55) return "rain";
  if (weatherCode <= 57) return "rain";
  if (weatherCode <= 65) return weatherCode >= 63 ? "heavyRain" : "rain";
  if (weatherCode <= 67) return "rain";
  if (weatherCode <= 77) return "snow";
  if (weatherCode >= 95) return "storm";
  if (weatherCode <= 82) return weatherCode >= 81 ? "heavyRain" : "rain";
  if (weatherCode <= 86) return "snow";
  return "cloudy";
}

/** dBZ radar color scale (Cumulus palette). */
export const DBZ_SCALE: { dbz: number; color: string }[] = [
  { dbz: 5,  color: "#7ae5a8" },
  { dbz: 15, color: "#3bc77a" },
  { dbz: 25, color: "#f5d042" },
  { dbz: 35, color: "#ff9f2e" },
  { dbz: 45, color: "#ff4040" },
  { dbz: 55, color: "#d02058" },
  { dbz: 65, color: "#b24bff" },
  { dbz: 75, color: "#ffffff" },
];

/** UV index → label + color. */
export function getUVInfo(uv: number): { label: string; color: string } {
  if (uv <= 2) return { label: "Low", color: cumulus.ok };
  if (uv <= 5) return { label: "Moderate", color: cumulus.sun };
  if (uv <= 7) return { label: "High", color: "#FF9F2E" };
  if (uv <= 10) return { label: "Very High", color: cumulus.hot };
  return { label: "Extreme", color: "#B24BFF" };
}

/** Wind speed → label + color. */
export function getWindInfo(mph: number): { label: string; color: string } {
  if (mph < 5) return { label: "Calm", color: cumulus.ok };
  if (mph < 15) return { label: "Light", color: cumulus.rain };
  if (mph < 25) return { label: "Moderate", color: cumulus.sun };
  if (mph < 35) return { label: "Strong", color: "#FF9F2E" };
  return { label: "Very Strong", color: cumulus.hot };
}

/** Wind direction degrees → 16-point compass label. */
export function getWindDirection(degrees: number): string {
  const dirs = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
  return dirs[Math.round(degrees / 22.5) % 16];
}
