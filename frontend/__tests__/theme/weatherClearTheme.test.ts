import {
  DARK_WEATHER_CLEAR_THEME,
  LIGHT_WEATHER_CLEAR_THEME,
  resolveAppearance,
  selectWeatherClearTheme,
} from "../../src/theme/weatherClearTheme";

describe("Weather Clear appearance", () => {
  it.each([
    ["light", "dark", "light"],
    ["dark", "light", "dark"],
    ["system", "dark", "dark"],
    ["system", "light", "light"],
    ["system", null, "light"],
  ] as const)("resolves %s with native %s to %s", (preference, native, expected) => {
    expect(resolveAppearance(preference, native)).toBe(expected);
  });

  it("uses the canonical paper and ink colors in light mode", () => {
    expect(LIGHT_WEATHER_CLEAR_THEME.colors.canvas).toBe("#f6f2ea");
    expect(LIGHT_WEATHER_CLEAR_THEME.colors.text).toBe("#211f1b");
    expect(LIGHT_WEATHER_CLEAR_THEME.colors.accent).toBe("#c2603a");
  });

  it("uses readable dark surfaces without changing semantic status colors", () => {
    expect(DARK_WEATHER_CLEAR_THEME.dark).toBe(true);
    expect(DARK_WEATHER_CLEAR_THEME.colors.canvas).toBe("#14130f");
    expect(DARK_WEATHER_CLEAR_THEME.colors.text).toBe("#f5efe4");
    expect(selectWeatherClearTheme("dark").colors.success).toBe("#56b97a");
  });
});
