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

/** 5-stop dark gradient per weather condition — hand-tuned from the prototype's CSS gradients. */
export type CumulusCondition =
  | "clearDay"
  | "clearNight"
  | "cloudy"
  | "rain"
  | "storm"
  | "snow"
  | "fog";

export const CONDITION_GRADIENTS: Record<CumulusCondition, readonly [string, string, string, string, string]> = {
  clearDay:   ["#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea"],
  clearNight: ["#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea"],
  cloudy:     ["#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea"],
  rain:       ["#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea"],
  storm:      ["#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea"],
  snow:       ["#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea"],
  fog:        ["#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea", "#f6f2ea"],
};

/** Map WMO weather code + day/night to Cumulus condition. */
export function getCumulusCondition(weatherCode: number, isNight: boolean): CumulusCondition {
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
  | "hail";

export function getIconKind(weatherCode: number, isNight: boolean): IconKind {
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
  if (weatherCode === 96 || weatherCode === 99) return "hail";
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

/** Determine night-time from sunrise/sunset (both as Date). */
export function isNightAt(now: Date, sunrise: Date, sunset: Date): boolean {
  return now < sunrise || now > sunset;
}
