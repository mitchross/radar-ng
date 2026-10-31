// Parsers for MMKV-persisted prefs. A stored value from an older build (or a
// typo'd server URL) must degrade to a default, never reach a lookup table and throw at render.
import type { MapStyle, Palette } from "../types/weather";

function oneOf<T extends string>(allowed: readonly T[], value: string, fallback: T): T {
  return (allowed as readonly string[]).includes(value) ? (value as T) : fallback;
}

export const MAP_STYLES = ["light", "dark", "satellite"] as const satisfies readonly MapStyle[];
export const PALETTES = ["classic", "vivid", "muted"] as const satisfies readonly Palette[];
export const VIEW_MODES = ["simple", "advanced"] as const;

export type ViewMode = (typeof VIEW_MODES)[number];

/** How fast radar is drawn: "skia" = canvas over the map (today), "in-map" = colored tiles as MapLibre rasters. */
export const RADAR_ENGINES = ["skia", "in-map"] as const;
export type RadarEngine = (typeof RADAR_ENGINES)[number];
export const DEFAULT_RADAR_ENGINE: RadarEngine = "skia";
/** Settings labels, in display order. */
export const RADAR_ENGINE_OPTIONS: readonly { value: RadarEngine; label: string }[] = [
  { value: "skia", label: "Classic fast" },
  { value: "in-map", label: "In-map" },
];

export const parseMapStyle = (v: string): MapStyle => oneOf(MAP_STYLES, v, "light");
export const parsePalette = (v: string): Palette => oneOf(PALETTES, v, "classic");
export const parseRadarEngine = (v: string): RadarEngine => oneOf(RADAR_ENGINES, v, DEFAULT_RADAR_ENGINE);
export const parseViewMode = (v: string): ViewMode => oneOf(VIEW_MODES, v, "simple");

/** Absolute http(s) origin without trailing slash; anything else falls back (tile URLs are string-joined onto it). */
export function parseServerUrl(v: string, fallback: string): string {
  const trimmed = v.trim().replace(/\/+$/, "");
  return /^https?:\/\/[^\s/]+(\/[^\s]*)?$/i.test(trimmed) ? trimmed : fallback;
}

/** Frames per second the playback timer honours. */
export const PLAYBACK_FPS_RANGE = { min: 1, max: 10 } as const;

/** Whole FPS clamped to PLAYBACK_FPS_RANGE; non-numeric → fallback. */
export function parsePlaybackFps(v: string, fallback: number): number {
  const n = Math.round(Number(v));
  if (v.trim() === "" || !Number.isFinite(n)) return fallback;
  return Math.max(PLAYBACK_FPS_RANGE.min, Math.min(PLAYBACK_FPS_RANGE.max, n));
}

/** Radar opacity in (0, 1]; NaN/out-of-range → fallback. */
export function parseOpacity(v: string, fallback: number): number {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 && n <= 1 ? n : fallback;
}
