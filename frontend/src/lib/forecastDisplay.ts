/**
 * Pure display helpers for the Home screen forecast layout.
 * Extracted from app/(tabs)/index.tsx — no React, no side effects.
 */
import type { OpenMeteoResponse } from "../types/weather";
import type { getCumulusCondition } from "./cumulusTheme";

export const CONDITION_LABELS: Record<ReturnType<typeof getCumulusCondition>, string> = {
  clearDay: "Sunny",
  clearNight: "Clear",
  cloudy: "Cloudy",
  rain: "Rain",
  storm: "Thunderstorms",
  snow: "Snow",
  fog: "Foggy",
};

export function findStartHourIndex(hours: string[]): number {
  const now = Date.now();
  for (let i = 0; i < hours.length; i++) {
    if (new Date(hours[i]).getTime() >= now - 30 * 60_000) return i;
  }
  return 0;
}

export function formatHour(d: Date, i: number): string {
  if (i === 0) return "NOW";
  const h = d.getHours();
  if (h === 0) return "12a";
  if (h === 12) return "12p";
  return h < 12 ? `${h}a` : `${h - 12}p`;
}

export function buildNowcastHeadline(
  minutely: OpenMeteoResponse["minutely_15"]
): { headline: string; sub: string } | null {
  if (!minutely || !minutely.precipitation || minutely.precipitation.length === 0) return null;
  const now = Date.now();
  const startIdx = minutely.time.findIndex((t) => new Date(t).getTime() >= now - 7.5 * 60_000);
  if (startIdx < 0) return null;
  const slice = minutely.precipitation.slice(startIdx, startIdx + 8);
  const firstWet = slice.findIndex((p) => p > 0.01);
  if (firstWet < 0) return null;
  const minutes = firstWet * 15;
  const continuedWet = slice.slice(firstWet).findIndex((p) => p < 0.005);
  const lastsMin = (continuedWet < 0 ? slice.length - firstWet : continuedWet) * 15;
  const total = slice.slice(firstWet).reduce((s, p) => s + Math.max(0, p), 0);
  const heavy = slice.slice(firstWet).some((p) => p > 0.3);
  const kind = heavy ? "Heavy rain" : "Rain";
  return {
    headline: minutes === 0 ? `${kind} now` : `${kind} starts in ${minutes} min`,
    sub: `Lasts ~${lastsMin} min \u00B7 ${total.toFixed(2)}" total`,
  };
}
