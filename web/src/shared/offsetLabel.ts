/**
 * A frame's offset from now for timeline labels, shared by the web radar and the app:
 * "Now" within 5 minutes (the same rule as the header), then "+45m", "−4h".
 */
export function offsetLabel(frameSec: number, nowSec: number): string {
  const min = Math.round((frameSec - nowSec) / 60);
  if (Math.abs(min) <= 5) return "Now";
  const sign = min > 0 ? "+" : "−";
  const abs = Math.abs(min);
  return abs < 90 ? `${sign}${abs}m` : `${sign}${Math.round(abs / 60)}h`;
}
