/**
 * Next-hour rain from the per-point MRMS nowcast: one interpolation and one
 * verdict for the web radar and the app (frontend/src/lib/radarNowcast.ts and
 * weatherPresentation.ts re-export these).
 */
import type { RadarNowcastPoint } from "./nowcastTypes";
import { RAIN_END_IN_HR, RAIN_START_IN_HR } from "./rainAlerts";

export type NowcastPoint = Pick<RadarNowcastPoint, "lead_minutes" | "precipitation_mm_h">;

/** Expand the server's five-minute MRMS samples into a per-minute series in inches/hour. */
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

export type NowcastVerdict =
  | { kind: "unavailable" }
  | { kind: "dry" }
  | { kind: "light" }
  | { kind: "raining"; peakMinute: number; endMinute: number }
  | { kind: "starting"; startMinute: number; peakMinute: number; endMinute: number };

export function getNowcastVerdict(values: number[] | undefined): NowcastVerdict {
  if (!values?.length) return { kind: "unavailable" };

  const startMinute = values.findIndex((value) => value > RAIN_START_IN_HR);
  if (startMinute < 0) return { kind: values.some((value) => value > 0) ? "light" : "dry" };

  const peakMinute = values.reduce((best, value, index) => (value > values[best] ? index : best), 0);
  let endMinute = values.length - 1;
  while (endMinute > startMinute && values[endMinute] <= RAIN_END_IN_HR) endMinute -= 1;

  return startMinute === 0
    ? { kind: "raining", peakMinute, endMinute }
    : { kind: "starting", startMinute, peakMinute, endMinute };
}

export function describeNowcast(verdict: NowcastVerdict): string {
  switch (verdict.kind) {
    case "unavailable":
      return "Next hour precipitation forecast unavailable.";
    case "dry":
      return "No rain expected for the next hour.";
    case "light":
      return "Light precipitation possible during the next hour.";
    case "raining":
      return `Raining now, peaks at ${verdict.peakMinute} minutes, and ends near ${verdict.endMinute} minutes.`;
    case "starting":
      return `Rain starts in ${verdict.startMinute} minutes, peaks at ${verdict.peakMinute} minutes, and ends near ${verdict.endMinute} minutes.`;
  }
}
