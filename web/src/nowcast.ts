/**
 * Next-hour rain from the per-point MRMS nowcast — same interpolation and verdict
 * rules as the app (frontend/src/lib/radarNowcast.ts, weatherPresentation.ts).
 */
export interface NowcastPoint {
  lead_minutes: number | null;
  precipitation_mm_h: number;
}

/** Per-minute intensity (inches/hour) for the next hour. */
export function interpolateNowcast(points: NowcastPoint[], horizon = 60): number[] {
  const ordered = points.filter((p) => p.lead_minutes != null).sort((a, b) => a.lead_minutes! - b.lead_minutes!);
  if (!ordered.length) return Array.from({ length: horizon }, () => 0);
  const anchors = [
    { minute: 0, v: ordered[0].precipitation_mm_h / 25.4 },
    ...ordered.map((p) => ({ minute: p.lead_minutes!, v: p.precipitation_mm_h / 25.4 })),
  ];
  return Array.from({ length: horizon }, (_, m) => {
    let upper = anchors.findIndex((a) => a.minute >= m);
    if (upper < 0) upper = anchors.length - 1;
    const lower = Math.max(0, upper - 1);
    const span = Math.max(1, anchors[upper].minute - anchors[lower].minute);
    const f = Math.max(0, Math.min(1, (m - anchors[lower].minute) / span));
    return Math.max(0, anchors[lower].v * (1 - f) + anchors[upper].v * f);
  });
}

export function nowcastVerdict(values: number[]): string {
  if (!values.length) return "Next hour precipitation forecast unavailable.";
  const start = values.findIndex((v) => v > 0.08);
  if (start < 0) return values.some((v) => v > 0) ? "Light precipitation possible during the next hour." : "No rain expected for the next hour.";
  const peak = values.reduce((best, v, i) => (v > values[best] ? i : best), 0);
  let end = values.length - 1;
  while (end > start && values[end] <= 0.05) end -= 1;
  return start === 0
    ? `Raining now, peaks at ${peak} minutes, and ends near ${end} minutes.`
    : `Rain starts in ${start} minutes, peaks at ${peak} minutes, and ends near ${end} minutes.`;
}
