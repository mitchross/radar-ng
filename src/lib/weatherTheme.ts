/**
 * Weather-adaptive theme system inspired by CARROT Weather.
 * Maps WMO weather codes to gradient backgrounds, personality quotes,
 * and visual properties. Includes time-of-day awareness.
 */

export interface WeatherTheme {
  gradient: [string, string, string];
  cardBg: string;
  cardBorder: string;
  textPrimary: string;
  textSecondary: string;
  accent: string;
}

type WeatherCategory =
  | "clear_day"
  | "clear_night"
  | "partly_cloudy_day"
  | "partly_cloudy_night"
  | "overcast"
  | "fog"
  | "drizzle"
  | "rain"
  | "heavy_rain"
  | "snow"
  | "thunderstorm";

const THEMES: Record<WeatherCategory, WeatherTheme> = {
  clear_day: {
    gradient: ["#1565C0", "#1E88E5", "#42A5F5"],
    cardBg: "rgba(255,255,255,0.15)",
    cardBorder: "rgba(255,255,255,0.25)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.7)",
    accent: "#FFD54F",
  },
  clear_night: {
    gradient: ["#0D1B2A", "#1B2838", "#2C3E50"],
    cardBg: "rgba(255,255,255,0.08)",
    cardBorder: "rgba(255,255,255,0.12)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.6)",
    accent: "#90CAF9",
  },
  partly_cloudy_day: {
    gradient: ["#37474F", "#546E7A", "#78909C"],
    cardBg: "rgba(255,255,255,0.12)",
    cardBorder: "rgba(255,255,255,0.18)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.65)",
    accent: "#4FC3F7",
  },
  partly_cloudy_night: {
    gradient: ["#1A237E", "#283593", "#3949AB"],
    cardBg: "rgba(255,255,255,0.08)",
    cardBorder: "rgba(255,255,255,0.12)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.6)",
    accent: "#7986CB",
  },
  overcast: {
    gradient: ["#455A64", "#607D8B", "#78909C"],
    cardBg: "rgba(255,255,255,0.10)",
    cardBorder: "rgba(255,255,255,0.15)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.6)",
    accent: "#B0BEC5",
  },
  fog: {
    gradient: ["#546E7A", "#78909C", "#90A4AE"],
    cardBg: "rgba(255,255,255,0.12)",
    cardBorder: "rgba(255,255,255,0.18)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.65)",
    accent: "#B0BEC5",
  },
  drizzle: {
    gradient: ["#37474F", "#455A64", "#607D8B"],
    cardBg: "rgba(255,255,255,0.10)",
    cardBorder: "rgba(255,255,255,0.15)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.6)",
    accent: "#4FC3F7",
  },
  rain: {
    gradient: ["#1A237E", "#1565C0", "#1976D2"],
    cardBg: "rgba(255,255,255,0.10)",
    cardBorder: "rgba(255,255,255,0.15)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.6)",
    accent: "#42A5F5",
  },
  heavy_rain: {
    gradient: ["#0D1B2A", "#1A237E", "#283593"],
    cardBg: "rgba(255,255,255,0.08)",
    cardBorder: "rgba(255,255,255,0.12)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.55)",
    accent: "#5C6BC0",
  },
  snow: {
    gradient: ["#546E7A", "#78909C", "#B0BEC5"],
    cardBg: "rgba(255,255,255,0.15)",
    cardBorder: "rgba(255,255,255,0.25)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.7)",
    accent: "#E0E0E0",
  },
  thunderstorm: {
    gradient: ["#1A0A2E", "#2D1B4E", "#4A148C"],
    cardBg: "rgba(255,255,255,0.08)",
    cardBorder: "rgba(255,255,255,0.12)",
    textPrimary: "#fff",
    textSecondary: "rgba(255,255,255,0.55)",
    accent: "#CE93D8",
  },
};

function categorizeWeather(code: number, isNight: boolean): WeatherCategory {
  if (code === 0) return isNight ? "clear_night" : "clear_day";
  if (code <= 2) return isNight ? "partly_cloudy_night" : "partly_cloudy_day";
  if (code === 3) return "overcast";
  if (code <= 48) return "fog";
  if (code <= 55) return "drizzle";
  if (code <= 65 || (code >= 80 && code <= 82)) return code >= 65 || code >= 82 ? "heavy_rain" : "rain";
  if (code <= 77 || (code >= 85 && code <= 86)) return "snow";
  if (code >= 95) return "thunderstorm";
  return "overcast";
}

export function getWeatherTheme(code: number, isNight?: boolean): WeatherTheme {
  const night = isNight ?? (new Date().getHours() >= 19 || new Date().getHours() < 6);
  const category = categorizeWeather(code, night);
  return THEMES[category];
}

/**
 * CARROT's signature: font weight varies with temperature.
 * Colder = thinner, warmer = bolder.
 */
export function getTempFontWeight(tempF: number): "100" | "200" | "300" | "400" | "500" | "600" | "700" | "800" | "900" {
  if (tempF < 10) return "100";
  if (tempF < 25) return "200";
  if (tempF < 40) return "300";
  if (tempF < 55) return "400";
  if (tempF < 70) return "500";
  if (tempF < 80) return "600";
  if (tempF < 90) return "700";
  if (tempF < 100) return "800";
  return "900";
}

/**
 * Color-code temperature values (blue for cold, red for hot).
 */
export function getTempColor(tempF: number): string {
  if (tempF < 0) return "#CE93D8";   // purple — extreme cold
  if (tempF < 20) return "#90CAF9";  // light blue
  if (tempF < 32) return "#4FC3F7";  // cyan
  if (tempF < 50) return "#4DD0E1";  // teal
  if (tempF < 65) return "#81C784";  // green
  if (tempF < 75) return "#FFD54F";  // yellow
  if (tempF < 85) return "#FFB74D";  // orange
  if (tempF < 95) return "#FF8A65";  // deep orange
  return "#EF5350";                   // red — extreme heat
}

// Snarky weather personality quotes (CARROT-style)
const QUIPS: Record<WeatherCategory, string[]> = {
  clear_day: [
    "The sun is being aggressively cheerful today.",
    "Enjoy this while it lasts, meatbag.",
    "Perfect weather for questioning your life choices outdoors.",
    "The sky is showing off. How predictable.",
    "Even I have to admit this is nice. Don't tell anyone.",
  ],
  clear_night: [
    "The stars are out. So romantic. So cold.",
    "Perfect night to stare into the void. It stares back.",
    "The moon is judging you. I can tell.",
    "Clear skies. The cosmos is watching your every move.",
  ],
  partly_cloudy_day: [
    "The clouds can't decide if they're coming or going.",
    "Partial commitment from the sky. How relatable.",
    "The sun is playing peek-a-boo. How childish.",
    "50% chance of sun. 100% chance of indecision.",
  ],
  partly_cloudy_night: [
    "The clouds are putting on a shadow puppet show.",
    "Mood lighting courtesy of partial cloud cover.",
    "The moon keeps disappearing. Very mysterious.",
  ],
  overcast: [
    "The sky has given up. Join the club.",
    "Gray. Gray everywhere. How inspiring.",
    "The sun called in sick today.",
    "Overcast. The weather equivalent of a Monday.",
    "The sky matches your personality: cloudy with zero sparkle.",
  ],
  fog: [
    "Can't see two feet ahead. Metaphor for your future.",
    "The atmosphere is being mysterious and unhelpful.",
    "Fog: nature's way of hiding its mistakes.",
    "Visibility is low. Just like your standards.",
  ],
  drizzle: [
    "It's barely raining. The sky is being passive-aggressive.",
    "Not enough rain to justify an umbrella. Just enough to ruin your hair.",
    "The clouds are spitting. How dignified.",
  ],
  rain: [
    "It's raining. I recommend being somewhere else.",
    "Water is falling from the sky. Groundbreaking.",
    "Perfect weather for dramatically staring out windows.",
    "The sky is crying. Probably because of you.",
    "Rain. Nature's way of saying 'stay home, nerd.'",
  ],
  heavy_rain: [
    "It's absolutely dumping out there. Stay inside, you fool.",
    "Noah called. He wants his flood back.",
    "The sky opened up and chose violence.",
    "If you go outside, you deserve what happens.",
  ],
  snow: [
    "It's snowing. How magical. How cold. How annoying.",
    "Winter wonderland? More like frozen wasteland.",
    "Snow: pretty for 5 minutes, miserable for 5 months.",
    "The sky is shedding. Gross.",
    "Bundle up, buttercup. It's frozen out there.",
  ],
  thunderstorm: [
    "THUNDER. I approve of nature's dramatic flair.",
    "The sky is having a tantrum. Relatable.",
    "Lightning! Finally, something exciting.",
    "Zeus is in a mood. Stay low, stay humble.",
    "The atmosphere chose violence today. Good for it.",
  ],
};

export function getWeatherQuip(code: number, isNight?: boolean): string {
  const night = isNight ?? (new Date().getHours() >= 19 || new Date().getHours() < 6);
  const category = categorizeWeather(code, night);
  const quotes = QUIPS[category];
  // Use day-of-year + hour so it changes but stays stable within the hour
  const now = new Date();
  const dayOfYear = Math.floor(
    (now.getTime() - new Date(now.getFullYear(), 0, 0).getTime()) / 86400000
  );
  const index = (dayOfYear * 7 + now.getHours()) % quotes.length;
  return quotes[index];
}
