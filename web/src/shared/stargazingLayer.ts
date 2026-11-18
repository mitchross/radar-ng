/**
 * The "Stargazing" map layer: the server scores each HRRR cell 0–100 for each
 * night hour from cloud cover, darkness and the moon (radar-grib stargazing.rs).
 * Day is transparent. Legend, readout and the card's best-window line, declared
 * once for the web radar and the app.
 */
import { clockLabel } from "./astro";
import type { ColorRange } from "./serverPalettes";

export const STARGAZING_LAYER = {
  id: "stargazing",
  name: "Stargazing",
  /** Server palette key (rust/crates/radar-tiler/assets/*.json). */
  paletteKey: "stargazing",
  hint: "Where the night sky is best: clouds, darkness and the moon",
  credit: "Our own index from NOAA HRRR clouds, the sun and the moon. No light-pollution data.",
} as const;

export type StargazingWord = "Excellent" | "Good" | "Fair" | "Poor";

/** Lower edge of each word, best first; matches the palette steps of 25. */
export const STARGAZING_WORDS: readonly (readonly [number, StargazingWord])[] = [
  [75, "Excellent"],
  [50, "Good"],
  [25, "Fair"],
  [0, "Poor"],
];

export function stargazingWord(score: number): StargazingWord {
  return (STARGAZING_WORDS.find(([min]) => score >= min) ?? STARGAZING_WORDS[STARGAZING_WORDS.length - 1])[1];
}

export interface StargazingLegendStop {
  word: StargazingWord;
  color: string;
}

/** Classic palette colors, used until the server palettes load. */
const FALLBACK: Record<StargazingWord, string> = {
  Excellent: "#966ee6",
  Good: "#3c78be",
  Fair: "#5f5f96",
  Poor: "#6e4646",
};

const hex = ([r, g, b]: ColorRange["rgba"]) => "#" + [r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("");

/** The four legend steps, best first, colored from the server's `stargazing` ranges when given. */
export function stargazingLegend(ranges?: readonly ColorRange[] | null): StargazingLegendStop[] {
  return STARGAZING_WORDS.map(([min, word]) => {
    const range = ranges?.find((r) => min >= r.min && min < r.max);
    return { word, color: range ? hex(range.rgba) : FALLBACK[word] };
  });
}

/** Inspector text for a map value: "Good · 62". */
export function formatStargazingReading(value: number | null | undefined): string | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  const score = Math.max(0, Math.min(100, Math.round(value)));
  return `${stargazingWord(score)} · ${score}`;
}

/** The server's best window tonight (`stargazing.best` in GET /api/sky/tonight). */
export interface StarWindow {
  start: number;
  end: number;
  score: number;
}

/** `stargazing` from the sky body: whether the layer is published, and its best window. */
export interface StargazingTonight {
  best: StarWindow | null;
}

export function parseStargazing(raw: unknown): StargazingTonight | null {
  const s = raw as { best?: unknown } | null | undefined;
  if (!s || typeof s !== "object") return null;
  const b = s.best as Record<string, unknown> | null | undefined;
  if (!b || typeof b !== "object") return { best: null };
  const start = typeof b.start === "string" ? Date.parse(b.start) : NaN;
  const end = typeof b.end === "string" ? Date.parse(b.end) : NaN;
  const score = typeof b.score === "number" ? b.score : NaN;
  if (![start, end, score].every(Number.isFinite) || end <= start) return { best: null };
  return { best: { start, end, score } };
}

/** Short clock for the line: "10 PM", or "10:30 PM" off the hour. */
function shortClock(ms: number, timeZone?: string, utcOffsetSeconds?: number): string {
  return clockLabel(ms, timeZone, utcOffsetSeconds).replace(":00", "");
}

/** "Best 10 PM – 2 AM · Excellent" ("Best until 2 AM" once it has begun), or null when over or missing. */
export function bestWindowLine(
  w: StarWindow | null | undefined,
  nowMs: number,
  clock: { timeZone?: string; utcOffsetSeconds?: number },
): string | null {
  if (!w || w.end <= nowMs) return null;
  const at = (ms: number) => shortClock(ms, clock.timeZone, clock.utcOffsetSeconds);
  const span = nowMs > w.start ? `until ${at(w.end)}` : `${at(w.start)} – ${at(w.end)}`;
  return `Best ${span} · ${stargazingWord(w.score)}`;
}
