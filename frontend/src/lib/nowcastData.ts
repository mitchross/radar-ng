/**
 * Pure data helpers for the Nowcast screen.
 * Extracted from screens/NowcastScreen.tsx — no React, no side effects.
 */

export type Minute = { i: number; intensity: number; confLo: number; confHi: number };

// Helper: build minute intervals
export function buildMinutes(
  minutely: { time: string[]; precipitation: number[] } | undefined,
): Minute[] {
  if (!minutely || minutely.precipitation.length === 0) {
    return Array.from({ length: 60 }, (_, i) => ({ i, intensity: 0, confLo: 0, confHi: 0 }));
  }
  const now = Date.now();
  const startIdx = Math.max(
    0,
    minutely.time.findIndex((t) => new Date(t).getTime() >= now - 7.5 * 60_000),
  );
  const quarters = minutely.precipitation.slice(startIdx, startIdx + 5);
  while (quarters.length < 5) quarters.push(0);

  const out: Minute[] = [];
  for (let i = 0; i < 60; i++) {
    const q = Math.min(3, Math.floor(i / 15));
    const frac = (i % 15) / 15;
    const intensity = Math.max(
      0,
      quarters[q] * (1 - frac) + quarters[q + 1] * frac,
    );
    const spread = 0.15 + i * 0.005;
    out.push({
      i,
      intensity,
      confLo: Math.max(0, intensity - spread),
      confHi: intensity + spread,
    });
  }
  return out;
}

export function estimateConfidence(forecast: { current: { time: string } }): number {
  const ageMin = (Date.now() - new Date(forecast.current.time).getTime()) / 60_000;
  if (ageMin < 5) return 0.95;
  if (ageMin < 15) return 0.8;
  if (ageMin < 30) return 0.65;
  return 0.5;
}
