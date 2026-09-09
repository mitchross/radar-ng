import type { TemperatureUnit } from "../types/weather";

/** The forecast API returns Fahrenheit; convert only at the display boundary. */
export function displayTemperature(fahrenheit: number, unit: TemperatureUnit): number {
  return Math.round(unit === "celsius" ? (fahrenheit - 32) * 5 / 9 : fahrenheit);
}
