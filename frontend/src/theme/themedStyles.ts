import { useWeatherClearTheme } from "./WeatherClearThemeProvider";
import type { WeatherClearTheme } from "./weatherClearTheme";

/**
 * One StyleSheet per theme, shared by every component instance in a file.
 * A per-component useMemo(createStyles) rebuilt the sheet for each list row.
 */
export function createThemedStyles<T>(factory: (theme: WeatherClearTheme) => T) {
  const cache = new WeakMap<WeatherClearTheme, T>();
  return function useThemedStyles(): { theme: WeatherClearTheme; styles: T } {
    const { theme } = useWeatherClearTheme();
    let styles = cache.get(theme);
    if (!styles) {
      styles = factory(theme);
      cache.set(theme, styles);
    }
    return { theme, styles };
  };
}
