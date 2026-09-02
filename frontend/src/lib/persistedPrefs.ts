// Parsers for MMKV-persisted prefs. A stored value from an older build (or a
// typo'd server URL) must degrade to a default, never reach a lookup table and throw at render.
import type { MapStyle, MapProjection, Palette, TimelineMode } from "../types/weather";

function oneOf<T extends string>(allowed: readonly T[], value: string, fallback: T): T {
  return (allowed as readonly string[]).includes(value) ? (value as T) : fallback;
}

export const MAP_STYLES = ["light", "dark", "satellite"] as const satisfies readonly MapStyle[];
export const MAP_PROJECTIONS = ["flat", "globe"] as const satisfies readonly MapProjection[];
export const PALETTES = ["classic", "vivid", "muted"] as const satisfies readonly Palette[];
export const TIMELINE_MODES = ["current", "forecast"] as const satisfies readonly TimelineMode[];
export const VIEW_MODES = ["simple", "advanced"] as const;

export type ViewMode = (typeof VIEW_MODES)[number];

export const parseMapStyle = (v: string): MapStyle => oneOf(MAP_STYLES, v, "light");
export const parseMapProjection = (v: string): MapProjection => oneOf(MAP_PROJECTIONS, v, "flat");
export const parsePalette = (v: string): Palette => oneOf(PALETTES, v, "classic");
export const parseTimelineMode = (v: string): TimelineMode => oneOf(TIMELINE_MODES, v, "forecast");
export const parseViewMode = (v: string): ViewMode => oneOf(VIEW_MODES, v, "simple");

/** Absolute http(s) origin without trailing slash; anything else falls back (tile URLs are string-joined onto it). */
export function parseServerUrl(v: string, fallback: string): string {
  const trimmed = v.trim().replace(/\/+$/, "");
  return /^https?:\/\/[^\s/]+(\/[^\s]*)?$/i.test(trimmed) ? trimmed : fallback;
}

/** Radar opacity in (0, 1]; NaN/out-of-range → fallback. */
export function parseOpacity(v: string, fallback: number): number {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 && n <= 1 ? n : fallback;
}
