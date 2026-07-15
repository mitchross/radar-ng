import type { RadarNowcastPoint } from "../types/weather";

/** Expand the server's five-minute MRMS samples into a smooth minute series. */
export function interpolateRadarNowcast(
  points: RadarNowcastPoint[],
  horizonMinutes = 60,
): number[] {
  const ordered = [...points]
    .filter((point) => point.lead_minutes != null)
    .sort((a, b) => (a.lead_minutes ?? 0) - (b.lead_minutes ?? 0));
  if (ordered.length === 0) return Array.from({ length: horizonMinutes }, () => 0);

  const anchors = [
    { minute: 0, intensity: ordered[0].precipitation_mm_h / 25.4 },
    ...ordered.map((point) => ({
      minute: point.lead_minutes ?? 0,
      // The screen's canonical intensity unit is inches/hour, matching the
      // self-hosted Open-Meteo proxy and its chart label.
      intensity: point.precipitation_mm_h / 25.4,
    })),
  ];
  return Array.from({ length: horizonMinutes }, (_, minute) => {
    let upper = anchors.findIndex((anchor) => anchor.minute >= minute);
    if (upper < 0) upper = anchors.length - 1;
    const lower = Math.max(0, upper - 1);
    const start = anchors[lower];
    const end = anchors[upper];
    const span = Math.max(1, end.minute - start.minute);
    const fraction = Math.max(0, Math.min(1, (minute - start.minute) / span));
    return Math.max(0, start.intensity * (1 - fraction) + end.intensity * fraction);
  });
}
