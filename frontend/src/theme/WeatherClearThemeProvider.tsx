import { createContext, use, useMemo, type ReactNode } from "react";
import { useColorScheme } from "react-native";
import { useWeatherStore } from "../stores/useWeatherStore";
import {
  resolveAppearance,
  selectWeatherClearTheme,
  type ResolvedAppearance,
  type WeatherClearTheme,
} from "./weatherClearTheme";

type ThemeContextValue = {
  theme: WeatherClearTheme;
  resolvedAppearance: ResolvedAppearance;
};

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function WeatherClearThemeProvider({ children }: { children: ReactNode }) {
  const preference = useWeatherStore((state) => state.appearanceMode);
  const nativeScheme = useColorScheme();
  const resolvedAppearance = resolveAppearance(preference, nativeScheme);
  const value = useMemo(
    () => ({
      resolvedAppearance,
      theme: selectWeatherClearTheme(resolvedAppearance),
    }),
    [resolvedAppearance],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useWeatherClearTheme(): ThemeContextValue {
  const value = use(ThemeContext);
  if (!value) {
    throw new Error("useWeatherClearTheme requires WeatherClearThemeProvider");
  }
  return value;
}
