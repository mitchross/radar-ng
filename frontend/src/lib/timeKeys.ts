/** ISO timestamp floored to the UTC hour, e.g. 2026-09-02T14:24:00Z → 2026-09-02T14:00:00Z. Returns input unchanged if unparseable. */
export function hourKey(iso: string): string {
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  const d = new Date(ms);
  d.setUTCMinutes(0, 0, 0);
  return d.toISOString().replace(".000Z", "Z");
}
