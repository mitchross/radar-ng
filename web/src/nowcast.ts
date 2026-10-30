/**
 * Next-hour rain from the per-point MRMS nowcast. Interpolation and verdict are
 * shared with the app (shared/nowcastVerdict.ts); only the card's headline lives here.
 */
import { rainHeadline } from "./shared/conditions";
import { describeNowcast, getNowcastVerdict } from "./shared/nowcastVerdict";
import { HEAVY_RAIN_IN_HR, RAIN_START_IN_HR } from "./shared/rainAlerts";

export { interpolateNowcast } from "./shared/nowcastVerdict";

export function nowcastVerdict(values: number[]): string {
  return describeNowcast(getNowcastVerdict(values));
}

/** The card's headline when rain is here or starting within the hour, as the app's rain banner says it. */
export function nextHourHeadline(values: number[]): string | null {
  const start = values.findIndex((v) => v > RAIN_START_IN_HR);
  if (start < 0) return null;
  return rainHeadline(values.some((v) => v > HEAVY_RAIN_IN_HR), start);
}
