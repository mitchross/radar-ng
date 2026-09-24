import type { TemperatureUnit } from "../types/weather";

/**
 * The forecast API returns Fahrenheit; convert only at the display boundary.
 * A missing value stays null — it must render as "—", never as 0°.
 */
export function displayTemperature(
  fahrenheit: number | null | undefined,
  unit: TemperatureUnit,
): number | null {
  if (fahrenheit == null || !Number.isFinite(fahrenheit)) return null;
  return Math.round(unit === "celsius" ? (fahrenheit - 32) * 5 / 9 : fahrenheit);
}

/** "72°", or "—" when the value is missing. */
export function formatDegrees(value: number | null): string {
  return value === null ? "\u2014" : `${value}\u00B0`;
}
