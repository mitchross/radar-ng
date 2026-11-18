/**
 * The "Nice Day" map layer: the server scores each HRRR cell 0–100 with the
 * same table as the card (niceDay.ts; the Rust twin reads the same vectors).
 * Legend, readout and credit, declared once for the web radar and the app.
 */
import { NICE_DAY_WORDS, wordFor, type NiceDayWord } from "./niceDay";
import type { ColorRange } from "./serverPalettes";

export const NICE_DAY_LAYER = {
  id: "nice-day",
  name: "Nice Day",
  /** Server palette key (rust/crates/radar-tiler/assets/*.json). */
  paletteKey: "nice_day",
  hint: "How nice each hour feels outside: temperature, wind, rain and humidity",
  credit: "Our own index from the NOAA HRRR forecast, not an official one.",
} as const;

export interface NiceDayLegendStop {
  word: NiceDayWord;
  color: string;
}

/** Classic palette colors, used until the server palettes load. */
const FALLBACK: Record<NiceDayWord, string> = {
  Great: "#32aa5a",
  Good: "#8cc85a",
  Fair: "#ebcd50",
  Poor: "#eb8c3c",
  "Do Not": "#c83c3c",
};

const hex = ([r, g, b]: ColorRange["rgba"]) => "#" + [r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("");

/**
 * The five legend steps, best first, colored from the server's `nice_day`
 * ranges when given (each range is one word, by its lower edge).
 */
export function niceDayLegend(ranges?: readonly ColorRange[] | null): NiceDayLegendStop[] {
  return NICE_DAY_WORDS.map(([min, word]) => {
    const range = ranges?.find((r) => min >= r.min && min < r.max);
    return { word, color: range ? hex(range.rgba) : FALLBACK[word] };
  });
}

/** Inspector text for a map value: "Good · 72". */
export function formatNiceDayReading(value: number | null | undefined): string | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  const score = Math.max(0, Math.min(100, Math.round(value)));
  return `${wordFor(score)} · ${score}`;
}
