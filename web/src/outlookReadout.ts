import type { FeatureCollection } from "geojson";
import { outlookDays } from "./shared/spc";
import { escapeHtml } from "./html";

const cache = new Map<number, { at: number; data: FeatureCollection }>();

export async function pointOutlookHtml(lon: number, lat: number): Promise<string> {
  const collections = await Promise.all([1, 2, 3].map(async (day) => {
    const hit = cache.get(day);
    if (hit && Date.now() - hit.at < 600_000) return hit.data;
    try {
      const res = await fetch(`/api/outlooks?day=${day}`, { signal: AbortSignal.timeout(10_000) });
      if (!res.ok) return null;
      const data = await res.json() as FeatureCollection;
      if (!Array.isArray(data.features)) return null;
      cache.set(day, { at: Date.now(), data });
      return data;
    } catch { return null; }
  }));
  return `<div class="outlook-readout"><b>SPC outlook</b>${outlookDays(collections, [lon, lat], Date.now()).map((line) => `<div>${escapeHtml(line)}</div>`).join("")}</div>`;
}
