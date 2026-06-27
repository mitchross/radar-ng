import type { ColorSchemeName } from "react-native";

export type AppearanceMode = "light" | "dark" | "system";
export type ResolvedAppearance = "light" | "dark";

export interface WeatherClearTheme {
  dark: boolean;
  colors: {
    canvas: string;
    surface: string;
    surfaceStrong: string;
    surfaceMuted: string;
    border: string;
    divider: string;
    text: string;
    textSecondary: string;
    textMuted: string;
    textFaint: string;
    accent: string;
    accentSoft: string;
    accentBorder: string;
    success: string;
    warning: string;
    destructive: string;
    rain: string;
    cold: string;
    hot: string;
    scrim: string;
  };
  typography: {
    display: string;
    displayItalic: string;
    ui: string;
    uiMedium: string;
    uiSemibold: string;
    uiBold: string;
    mono: string;
  };
  spacing: { xs: 4; sm: 8; md: 12; lg: 16; xl: 24; xxl: 32 };
  radii: { sm: 8; md: 12; lg: 18; xl: 22; pill: 999 };
  controlMinSize: 44;
}

const isIOS = process.env.EXPO_OS === "ios";
const font = (android: string, ios: string): string => (isIOS ? ios : android);

const shared = {
  spacing: { xs: 4, sm: 8, md: 12, lg: 16, xl: 24, xxl: 32 },
  radii: { sm: 8, md: 12, lg: 18, xl: 22, pill: 999 },
  controlMinSize: 44,
  typography: {
    display: font("Newsreader_400Regular", "Newsreader-Regular"),
    displayItalic: font("Newsreader_400Regular_Italic", "Newsreader-Italic"),
    ui: font("SplineSans_400Regular", "SplineSans-Regular"),
    uiMedium: font("SplineSans_500Medium", "SplineSans-Medium"),
    uiSemibold: font("SplineSans_600SemiBold", "SplineSans-SemiBold"),
    uiBold: font("SplineSans_700Bold", "SplineSans-Bold"),
    mono: "monospace",
  },
} as const;

export const LIGHT_WEATHER_CLEAR_THEME: WeatherClearTheme = {
  ...shared,
  dark: false,
  colors: {
    canvas: "#f6f2ea",
    surface: "#ffffff",
    surfaceStrong: "#fbf9f5",
    surfaceMuted: "#eae4d8",
    border: "#eee6d8",
    divider: "#e7e0d3",
    text: "#211f1b",
    textSecondary: "#5d574d",
    textMuted: "#8c857a",
    textFaint: "#a39a8a",
    accent: "#c2603a",
    accentSoft: "rgba(194,96,58,0.12)",
    accentBorder: "rgba(194,96,58,0.30)",
    success: "#56b97a",
    warning: "#f0c34e",
    destructive: "#df6a6a",
    rain: "#4d7fb8",
    cold: "#6db4d8",
    hot: "#df6a3c",
    scrim: "rgba(33,31,27,0.45)",
  },
};

export const DARK_WEATHER_CLEAR_THEME: WeatherClearTheme = {
  ...shared,
  dark: true,
  colors: {
    canvas: "#14130f",
    surface: "#1d1b16",
    surfaceStrong: "#242119",
    surfaceMuted: "#302c24",
    border: "#3a352b",
    divider: "#332f27",
    text: "#f5efe4",
    textSecondary: "#c9bdac",
    textMuted: "#a79b89",
    textFaint: "#81786b",
    accent: "#d9825d",
    accentSoft: "rgba(217,130,93,0.16)",
    accentBorder: "rgba(217,130,93,0.36)",
    success: "#56b97a",
    warning: "#e8bd55",
    destructive: "#e47d7d",
    rain: "#76a2d4",
    cold: "#79bddb",
    hot: "#e47a50",
    scrim: "rgba(0,0,0,0.62)",
  },
};

export function resolveAppearance(
  preference: AppearanceMode,
  nativeScheme: ColorSchemeName | null,
): ResolvedAppearance {
  if (preference !== "system") return preference;
  return nativeScheme === "dark" ? "dark" : "light";
}

export function selectWeatherClearTheme(
  resolved: ResolvedAppearance,
): WeatherClearTheme {
  return resolved === "dark"
    ? DARK_WEATHER_CLEAR_THEME
    : LIGHT_WEATHER_CLEAR_THEME;
}
