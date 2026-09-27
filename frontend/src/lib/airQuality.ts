/**
 * US AQI from PM2.5 and frame selection for the home Air Quality tile. The
 * NOAA AQM grids behind the air-quality/ozone map layers are sampled at a
 * point by the tile-server's /api/inspect route.
 */

/** EPA PM2.5 breakpoints (µg/m³, 24-h, 2024 revision) → AQI. */
const PM25_BREAKPOINTS: [number, number, number, number][] = [
  [0, 9.0, 0, 50],
  [9.1, 35.4, 51, 100],
  [35.5, 55.4, 101, 150],
  [55.5, 125.4, 151, 200],
  [125.5, 225.4, 201, 300],
  [225.5, 325.4, 301, 500],
];

export function usAqiFromPm25(ugm3: number): number {
  const c = Math.max(0, Math.floor(ugm3 * 10) / 10);
  for (const [cLo, cHi, iLo, iHi] of PM25_BREAKPOINTS) {
    if (c <= cHi) return Math.round(((iHi - iLo) / (cHi - cLo)) * (Math.max(c, cLo) - cLo) + iLo);
  }
  return 500;
}

export function aqiCategory(aqi: number): { label: string; color: string } {
  if (aqi <= 50) return { label: "Good", color: "#3bb273" };
  if (aqi <= 100) return { label: "Moderate", color: "#e0b400" };
  if (aqi <= 150) return { label: "Unhealthy for sensitive groups", color: "#ff8c2e" };
  if (aqi <= 200) return { label: "Unhealthy", color: "#e5484d" };
  if (aqi <= 300) return { label: "Very unhealthy", color: "#8f3f97" };
  return { label: "Hazardous", color: "#7e0023" };
}

/**
 * The frame valid nearest to `now`, or null when none is within `maxGapMs`
 * (a stale AQM run must not pass for current air quality).
 */
export function nearestFrameTimestamp(
  frames: readonly { timestamp: string }[] | undefined,
  now: number,
  maxGapMs = 90 * 60_000,
): string | null {
  let best: { ts: string; gap: number } | null = null;
  for (const f of frames ?? []) {
    const t = Date.parse(f.timestamp);
    if (!Number.isFinite(t)) continue;
    const gap = Math.abs(t - now);
    if (!best || gap < best.gap) best = { ts: f.timestamp, gap };
  }
  return best && best.gap <= maxGapMs ? best.ts : null;
}
