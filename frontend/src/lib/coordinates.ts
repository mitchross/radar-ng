/**
 * The one place where a device coordinate is narrowed before it leaves the app.
 *
 * Every outbound request that carries a position — the forecast/nowcast paths,
 * the storm-prefetch query, the inspector point, and the NWS `point=` parameter
 * — must use a value from here, and so must every react-query key that depends
 * on position. Keeping both on the same rounded value is what stops GPS jitter
 * from creating a new cache entry (and a new request) every few metres.
 *
 * Rounding costs no accuracy. MRMS radar and the self-hosted nowcast grid are
 * 1 km cells and the Open-Meteo models behind the forecast are 1–11 km, so all
 * of them are already coarser than a 2-decimal fix (≈1.1 km at the equator).
 * NWS alerts are the exception: their polygons can be far smaller than a
 * forecast cell, so they keep 3 decimals (≈110 m) to avoid rounding a point
 * across a warning boundary.
 */
export const PRECISION = {
  /** ≈1.1 km — forecast, nowcast, storm prefetch. */
  WEATHER: 2,
  /** ≈110 m — NWS alerts and the inspector sample point. */
  POINT: 3,
} as const;

/** Degrees of latitude/longitude per decimal place, for docs and tests. */
export const METRES_PER_DEGREE_AT_EQUATOR = 111_320;

export function roundCoord(value: number, decimals: number): number {
  const factor = 10 ** decimals;
  return Math.round(value * factor) / factor;
}

export interface Coords {
  lat: number;
  lon: number;
}

export function roundCoords(lat: number, lon: number, decimals: number): Coords {
  return { lat: roundCoord(lat, decimals), lon: roundCoord(lon, decimals) };
}

/**
 * Round a nullable position pair, for callers that hold latitude/longitude
 * separately and may not have a fix yet. Returns null unless both are present.
 */
export function roundNullableCoords(
  lat: number | null | undefined,
  lon: number | null | undefined,
  decimals: number,
): Coords | null {
  if (lat == null || lon == null) return null;
  return roundCoords(lat, lon, decimals);
}

/** Stable react-query key segment for a rounded position, e.g. "43.23,-85.55". */
export function locationKey(lat: number, lon: number, decimals: number): string {
  const { lat: rLat, lon: rLon } = roundCoords(lat, lon, decimals);
  return `${rLat},${rLon}`;
}
