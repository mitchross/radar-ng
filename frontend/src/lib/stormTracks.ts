/**
 * Storm-cell tracks and "arriving in N min", same maths as web/src/overlays.ts
 * so the site and the app draw and warn identically.
 */
import {
  advance,
  distanceKm,
  incomingStorm,
  isTracked,
  stormConeFeatures,
  TRACK_MINUTES,
  type IncomingStorm,
  type StormCell,
} from "../../../web/src/shared/stormCone";

// Track rule, cone, incoming-storm search and the approach notice live in one
// module shared with the web radar. Only the app's parsing and wording stay here.
export { advance, distanceKm, incomingStorm, isTracked, TRACK_MINUTES };
export type { IncomingStorm };

const KMH_TO_MPH = 0.621371;

export type StormCellInfo = StormCell;

type Collection = { features: { geometry: { type: string; coordinates: unknown }; properties?: Record<string, unknown> | null }[] };

export function parseStormCells(fc: Collection | undefined | null): StormCellInfo[] {
  if (!fc?.features) return [];
  return fc.features.flatMap((f) => {
    if (f.geometry?.type !== "Point") return [];
    const [lon, lat] = f.geometry.coordinates as [number, number];
    const p = (f.properties ?? {}) as Record<string, any>;
    const v = p.tracking_vector ?? {};
    return [{
      id: Number(p.cell_id),
      lon,
      lat,
      peakDbz: Number(p.peak_dbz ?? 0),
      areaKm2: Number(p.area_km2 ?? 0),
      speedMph: Number(v.speed_kmh ?? 0) * KMH_TO_MPH,
      bearing: Number(v.bearing_deg ?? 0),
      confidence: Number(p.tracking_confidence ?? 0),
      eastKmh: Number(v.east_kmh ?? 0),
      northKmh: Number(v.north_kmh ?? 0),
      history: Array.isArray(p.track_history) ? p.track_history : [],
    }];
  });
}

export function stormTrackGeoJSON(cells: StormCellInfo[]) {
  const tracks: GeoJSON.Feature[] = [];
  const ticks: GeoJSON.Feature[] = [];
  const past: GeoJSON.Feature[] = [];
  for (const c of cells) {
    if (c.history.length >= 2) {
      past.push({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: c.history.map((h) => [h[1], h[2]]) } });
    }
    if (!isTracked(c)) continue;
    const pts = [[c.lon, c.lat], ...TRACK_MINUTES.map((m) => advance(c.lon, c.lat, c.eastKmh, c.northKmh, m))];
    tracks.push({ type: "Feature", properties: { severe: c.peakDbz >= 55 }, geometry: { type: "LineString", coordinates: pts } });
    TRACK_MINUTES.forEach((m, i) =>
      ticks.push({ type: "Feature", properties: { label: String(m) }, geometry: { type: "Point", coordinates: pts[i + 1] } }),
    );
  }
  const fc = (features: GeoJSON.Feature[]): GeoJSON.FeatureCollection => ({ type: "FeatureCollection", features });
  return { tracks: fc(tracks), ticks: fc(ticks), past: fc(past), cones: fc(stormConeFeatures(cells) as GeoJSON.Feature[]) };
}

export const compass = (deg: number) => ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][Math.round((((deg % 360) + 360) % 360) / 45) % 8];

export function hailLabel(peakDbz: number): string | null {
  if (peakDbz >= 60) return "Hail likely";
  if (peakDbz >= 55) return "Hail possible";
  return null;
}

/** "Hail-producing storm arriving in ~20 min", shared wording for the Home card. */
export function describeIncoming(t: IncomingStorm): { headline: string; sub: string; severe: boolean } {
  const what = t.peakDbz >= 60 ? "Hail-producing storm" : t.peakDbz >= 50 ? "Heavy storm" : "Rain";
  return {
    headline: `${what} ${t.minutes === 0 ? "over you now" : `arriving in ~${t.minutes} min`}`,
    sub: `${Math.round(t.peakDbz)} dBZ · moving ${compass(t.bearing)} at ${Math.round(t.speedMph)} mph`,
    severe: t.peakDbz >= 55,
  };
}
