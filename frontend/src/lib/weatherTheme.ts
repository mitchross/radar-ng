/**
 * Weather-adaptive theme system inspired by CARROT Weather Premium.
 * Maps WMO weather codes to rich gradient backgrounds, personality quotes,
 * scene illustration config, and visual properties.
 * Includes time-of-day awareness with dramatic multi-stop gradients.
 */

export type SceneType =
  | "sunny"
  | "night_clear"
  | "cloudy"
  | "night_cloudy"
  | "overcast"
  | "foggy"
  | "rainy"
  | "stormy"
  | "snowy"
  | "thunderstorm";

export interface WeatherTheme {
  gradient: [string, string, string, string, string];
  cardBg: string;
  cardBorder: string;
  textPrimary: string;
  textSecondary: string;
  accent: string;
  sceneType: SceneType;
  /** Colors for the skyline/scene illustration */
  scene: {
    skyline: string;
    skylineLight: string;
    celestial: string;
    celestialGlow: string;
    particles: string;
  };
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
    gradient: ["#0A47A0", "#1565C0", "#1E88E5", "#42A5F5", "#64B5F6"],
    cardBg: "rgba(255,255,255,0.12)",
    cardBorder: "rgba(255,255,255,0.20)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.75)",
    accent: "#FFD54F",
    sceneType: "sunny",
    scene: {
      skyline: "rgba(0,40,100,0.6)",
      skylineLight: "rgba(0,60,140,0.3)",
      celestial: "#FFD54F",
      celestialGlow: "rgba(255,213,79,0.3)",
      particles: "rgba(255,255,255,0.1)",
    },
  },
  clear_night: {
    gradient: ["#050A18", "#0D1B2A", "#152238", "#1B2D4A", "#243B5C"],
    cardBg: "rgba(255,255,255,0.07)",
    cardBorder: "rgba(255,255,255,0.10)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.60)",
    accent: "#90CAF9",
    sceneType: "night_clear",
    scene: {
      skyline: "rgba(10,15,30,0.8)",
      skylineLight: "rgba(20,30,60,0.4)",
      celestial: "#E0E0E0",
      celestialGlow: "rgba(200,200,255,0.15)",
      particles: "rgba(255,255,255,0.6)",
    },
  },
  partly_cloudy_day: {
    gradient: ["#2C5282", "#3A6EA5", "#5A8AB5", "#7BA3C7", "#9BBCD8"],
    cardBg: "rgba(255,255,255,0.10)",
    cardBorder: "rgba(255,255,255,0.16)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.68)",
    accent: "#4FC3F7",
    sceneType: "cloudy",
    scene: {
      skyline: "rgba(30,50,80,0.6)",
      skylineLight: "rgba(40,70,110,0.3)",
      celestial: "#FFE082",
      celestialGlow: "rgba(255,224,130,0.2)",
      particles: "rgba(255,255,255,0.15)",
    },
  },
  partly_cloudy_night: {
    gradient: ["#0A0E2A", "#151B4A", "#1F2768", "#2A3480", "#354196"],
    cardBg: "rgba(255,255,255,0.07)",
    cardBorder: "rgba(255,255,255,0.10)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.58)",
    accent: "#7986CB",
    sceneType: "night_cloudy",
    scene: {
      skyline: "rgba(10,12,35,0.8)",
      skylineLight: "rgba(15,20,50,0.4)",
      celestial: "#C5CAE9",
      celestialGlow: "rgba(180,190,230,0.12)",
      particles: "rgba(255,255,255,0.4)",
    },
  },
  overcast: {
    gradient: ["#37474F", "#455A64", "#546E7A", "#607D8B", "#78909C"],
    cardBg: "rgba(255,255,255,0.09)",
    cardBorder: "rgba(255,255,255,0.13)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.60)",
    accent: "#B0BEC5",
    sceneType: "overcast",
    scene: {
      skyline: "rgba(40,50,55,0.7)",
      skylineLight: "rgba(55,70,80,0.35)",
      celestial: "#90A4AE",
      celestialGlow: "rgba(144,164,174,0.15)",
      particles: "rgba(255,255,255,0.08)",
    },
  },
  fog: {
    gradient: ["#455A64", "#607D8B", "#78909C", "#90A4AE", "#B0BEC5"],
    cardBg: "rgba(255,255,255,0.10)",
    cardBorder: "rgba(255,255,255,0.16)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.65)",
    accent: "#B0BEC5",
    sceneType: "foggy",
    scene: {
      skyline: "rgba(60,75,85,0.5)",
      skylineLight: "rgba(80,100,115,0.25)",
      celestial: "#CFD8DC",
      celestialGlow: "rgba(207,216,220,0.2)",
      particles: "rgba(255,255,255,0.25)",
    },
  },
  drizzle: {
    gradient: ["#263238", "#37474F", "#455A64", "#546E7A", "#607D8B"],
    cardBg: "rgba(255,255,255,0.08)",
    cardBorder: "rgba(255,255,255,0.12)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.60)",
    accent: "#4FC3F7",
    sceneType: "rainy",
    scene: {
      skyline: "rgba(30,40,45,0.7)",
      skylineLight: "rgba(40,55,65,0.35)",
      celestial: "#78909C",
      celestialGlow: "rgba(120,144,156,0.1)",
      particles: "rgba(130,180,220,0.6)",
    },
  },
  rain: {
    gradient: ["#0D2137", "#153050", "#1A4068", "#1E5080", "#236098"],
    cardBg: "rgba(255,255,255,0.08)",
    cardBorder: "rgba(255,255,255,0.12)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.60)",
    accent: "#42A5F5",
    sceneType: "rainy",
    scene: {
      skyline: "rgba(10,25,45,0.7)",
      skylineLight: "rgba(15,35,60,0.35)",
      celestial: "#546E7A",
      celestialGlow: "rgba(84,110,122,0.1)",
      particles: "rgba(100,160,220,0.7)",
    },
  },
  heavy_rain: {
    gradient: ["#0A1628", "#0D1F3C", "#102850", "#133264", "#163C78"],
    cardBg: "rgba(255,255,255,0.06)",
    cardBorder: "rgba(255,255,255,0.10)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.55)",
    accent: "#5C6BC0",
    sceneType: "stormy",
    scene: {
      skyline: "rgba(8,15,30,0.8)",
      skylineLight: "rgba(12,22,42,0.4)",
      celestial: "#455A64",
      celestialGlow: "rgba(69,90,100,0.08)",
      particles: "rgba(80,140,200,0.8)",
    },
  },
  snow: {
    gradient: ["#3E5060", "#526878", "#688090", "#7E98A8", "#94B0C0"],
    cardBg: "rgba(255,255,255,0.12)",
    cardBorder: "rgba(255,255,255,0.20)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.70)",
    accent: "#E0E0E0",
    sceneType: "snowy",
    scene: {
      skyline: "rgba(45,60,70,0.6)",
      skylineLight: "rgba(60,80,95,0.3)",
      celestial: "#CFD8DC",
      celestialGlow: "rgba(207,216,220,0.2)",
      particles: "rgba(255,255,255,0.8)",
    },
  },
  thunderstorm: {
    gradient: ["#0A0518", "#1A0A2E", "#2D1548", "#3D1F62", "#4A2878"],
    cardBg: "rgba(255,255,255,0.07)",
    cardBorder: "rgba(255,255,255,0.10)",
    textPrimary: "#FFFFFF",
    textSecondary: "rgba(255,255,255,0.55)",
    accent: "#CE93D8",
    sceneType: "thunderstorm",
    scene: {
      skyline: "rgba(10,5,20,0.8)",
      skylineLight: "rgba(20,10,40,0.4)",
      celestial: "#7E57C2",
      celestialGlow: "rgba(126,87,194,0.15)",
      particles: "rgba(200,180,255,0.6)",
    },
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

/**
 * Wind speed to descriptive label and color.
 */
export function getWindInfo(mph: number): { label: string; color: string } {
  if (mph < 5) return { label: "Calm", color: "#81C784" };
  if (mph < 15) return { label: "Light", color: "#4FC3F7" };
  if (mph < 25) return { label: "Moderate", color: "#FFD54F" };
  if (mph < 35) return { label: "Strong", color: "#FFB74D" };
  if (mph < 50) return { label: "Very Strong", color: "#FF8A65" };
  return { label: "Extreme", color: "#EF5350" };
}

/**
 * UV index to risk level and color.
 */
export function getUVInfo(uv: number): { label: string; color: string } {
  if (uv <= 2) return { label: "Low", color: "#81C784" };
  if (uv <= 5) return { label: "Moderate", color: "#FFD54F" };
  if (uv <= 7) return { label: "High", color: "#FFB74D" };
  if (uv <= 10) return { label: "Very High", color: "#FF8A65" };
  return { label: "Extreme", color: "#EF5350" };
}

/**
 * Wind direction degrees to compass label.
 */
export function getWindDirection(degrees: number): string {
  const dirs = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
  const index = Math.round(degrees / 22.5) % 16;
  return dirs[index];
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
