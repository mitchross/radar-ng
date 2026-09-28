/**
 * Pure derivations for the Advanced "Conditions" tiles. Inputs are the
 * forecast API's units (°F, mph, hPa, inches); a missing input stays null.
 */
import { startHourIndex } from "./forecastView";
import type { OpenMeteoResponse } from "../types/weather";

const HPA_PER_INHG = 33.8639;

export function hPaToInHg(hPa: number): number {
  return Math.round((hPa / HPA_PER_INHG) * 100) / 100;
}

export function pressureLabel(hPa: number): "Low" | "Normal" | "High" {
  if (hPa < 1009) return "Low";
  if (hPa > 1022) return "High";
  return "Normal";
}

/** Bearing the wind blows toward: the API gives the direction it comes from. */
export function windToward(fromDegrees: number): number {
  return (((fromDegrees + 180) % 360) + 360) % 360;
}

/** Dew point in °F, the measure of how muggy the air feels. */
export function dewPointFeel(dewF: number): string {
  if (dewF < 55) return "Comfortable";
  if (dewF < 65) return "Sticky";
  return "Oppressive";
}

/** Why the feels-like temperature differs from the air temperature. */
export function feelsLikeNote(actualF: number, feelsF: number): string {
  const diff = feelsF - actualF;
  if (Math.abs(diff) < 3) return "Similar to the actual temperature";
  return diff < 0 ? "Wind makes it feel colder" : "Humidity makes it feel warmer";
}

/** Hourly precipitation amounts (inches) for the next `hours` hours; null where unknown. */
export function hourlyPrecipitation(
  forecast: OpenMeteoResponse,
  now = Date.now(),
  hours = 24,
): (number | null)[] {
  const start = startHourIndex(forecast.hourly.time, now);
  return Array.from({ length: hours }, (_, i) => {
    const v = forecast.hourly.precipitation[start + i];
    return v == null || !Number.isFinite(v) ? null : Math.max(0, v);
  });
}

function duration(minutes: number): string {
  const h = Math.floor(minutes / 60);
  const m = Math.round(minutes % 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/** "5h 28m of daylight left", "Sunrise in 2h 3m", or null without sun times. */
export function daylightNote(now: Date, sunrise: Date | null, sunset: Date | null): string | null {
  if (!sunrise || !sunset) return null;
  const t = now.getTime();
  if (t < sunrise.getTime()) return `Sunrise in ${duration((sunrise.getTime() - t) / 60_000)}`;
  if (t < sunset.getTime()) return `${duration((sunset.getTime() - t) / 60_000)} of daylight left`;
  return `${duration((sunset.getTime() - sunrise.getTime()) / 60_000)} of daylight today`;
}
