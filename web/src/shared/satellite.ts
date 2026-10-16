/**
 * GOES satellite frames (the manifest `satellite` layer, served from NASA GIBS
 * through `/tiles/satellite/...`) matched to the radar timeline. The web and
 * the app draw the satellite frame at or before the timeline time, under the
 * radar. Pure, so both clients share it.
 */

export interface SatelliteFrame {
  /** Epoch seconds. */
  time: number;
  /** Tile path segment, e.g. `2026-10-15T22:10:00Z`. */
  path: string;
  maxZoom: number;
}

interface ManifestLike {
  layers: Record<string, { frames?: { timestamp: string; path: string; max_zoom?: number }[] } | undefined>;
}

/** GIBS GoogleMapsCompatible_Level7 stops at z7; MapLibre overzooms past it. */
export const SATELLITE_MAX_ZOOM = 7;
/** GeoColor is opaque imagery; a little basemap shows through for borders and roads. */
export const SATELLITE_OPACITY = 0.85;
/**
 * The newest satellite frame is ~40 min old at "now". Past this age at the
 * timeline time (deep into the forecast) the layer hides instead of passing
 * an old image off as current.
 */
export const SATELLITE_MAX_AGE_S = 90 * 60;

/** The manifest's satellite frames, oldest first. Empty when the server has none. */
export function satelliteFrames(manifest: ManifestLike | null | undefined): SatelliteFrame[] {
  return (manifest?.layers.satellite?.frames ?? [])
    .map((f) => ({ time: Math.floor(Date.parse(f.timestamp) / 1000), path: f.path, maxZoom: f.max_zoom ?? SATELLITE_MAX_ZOOM }))
    .filter((f) => Number.isFinite(f.time))
    .sort((a, b) => a.time - b.time);
}

/** Index of the newest frame at or before `timeSec` and no older than SATELLITE_MAX_AGE_S, else -1. */
export function satelliteIndexAt(frames: readonly SatelliteFrame[], timeSec: number): number {
  for (let i = frames.length - 1; i >= 0; i--) {
    if (frames[i].time <= timeSec) return timeSec - frames[i].time <= SATELLITE_MAX_AGE_S ? i : -1;
  }
  return -1;
}

/** The distinct satellite frames a playback order visits, in that order (for prefetch). */
export function satelliteSequence(frames: readonly SatelliteFrame[], timelineTimes: readonly number[]): number[] {
  const seen = new Set<number>();
  for (const t of timelineTimes) {
    const i = satelliteIndexAt(frames, t);
    if (i >= 0) seen.add(i);
  }
  return [...seen];
}

/** Raster tile URL template for one frame. The path is server-validated (`[0-9TZ:-]`), so it is not encoded. */
export function satelliteTileUrl(origin: string, path: string): string {
  return `${origin}/tiles/satellite/${path}/{z}/{x}/{y}.png`;
}
