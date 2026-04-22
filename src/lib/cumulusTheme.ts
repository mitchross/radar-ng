/**
 * Cumulus design system — ported from UI-Handoff/design_handoff_cumulus_weather/src/design.js.
 * Single violet accent, condition-driven dark gradient backgrounds, dBZ radar scale.
 */

export const cumulus = {
  accent: "#8B7CFF",
  accentBright: "#A594FF",
  accentDim: "#5B4FD6",
  accentSoft: "rgba(139,124,255,0.18)",
  accentBorder: "rgba(139,124,255,0.55)",

  ink: "#FFFFFF",
  inkDim: "rgba(255,255,255,0.72)",
  inkMuted: "rgba(255,255,255,0.48)",
  inkFaint: "rgba(255,255,255,0.28)",
  inkLine: "rgba(255,255,255,0.10)",

  card: "rgba(255,255,255,0.06)",
  cardStrong: "rgba(255,255,255,0.10)",
  cardLine: "rgba(255,255,255,0.08)",

  rain: "#4FB8FF",
  rainHeavy: "#1E7FFF",
  snow: "#C7E6FF",
  sun: "#FFC14D",
  temp: "#FF6E3A",
  cold: "#5BD4FF",
  hot: "#FF4D6D",
  alert: "#FF3B4A",
  ok: "#4ADE80",
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
  clearDay:   ["#5B8FFF", "#3E5DE0", "#2A3FA8", "#1E2670", "#140E3D"],
  clearNight: ["#2B2060", "#1F1750", "#150B3D", "#0B0725", "#050316"],
  cloudy:     ["#3F4A6B", "#2F3858", "#1E2540", "#151A30", "#0C1020"],
  rain:       ["#2C5076", "#234262", "#18304F", "#101E38", "#070D1C"],
  storm:      ["#5B2A7A", "#442060", "#321551", "#1D0C30", "#0A0418"],
  snow:       ["#4A5B78", "#384A66", "#26324E", "#161F34", "#0B1020"],
  fog:        ["#4A5566", "#3A4350", "#2A313F", "#1C222C", "#0F131A"],
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
