/**
 * Storm-cell tracks and "arriving in N min" — same maths as web/src/overlays.ts
 * so the site and the app draw and warn identically.
 */
const KMH_TO_MPH = 0.621371;
export const TRACK_MINUTES = [15, 30, 45, 60];
const MIN_TRACK_CONFIDENCE = 0.4;

export interface StormCellInfo {
  id: number;
  lon: number;
  lat: number;
  peakDbz: number;
  areaKm2: number;
  speedMph: number;
  bearing: number;
  confidence: number;
  eastKmh: number;
  northKmh: number;
  history: [number, number, number][];
}

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

export function advance(lon: number, lat: number, eastKmh: number, northKmh: number, minutes: number): [number, number] {
  const h = minutes / 60;
  return [lon + (eastKmh * h) / Math.max(1e-6, 111.32 * Math.cos((lat * Math.PI) / 180)), lat + (northKmh * h) / 110.574];
}

/** History-backed fits only: one-step vectors from older servers were noise. */
export const isTracked = (c: StormCellInfo) =>
  c.history.length >= 3 && c.confidence >= MIN_TRACK_CONFIDENCE && c.speedMph >= 3 && c.speedMph <= 90;

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
  return { tracks: fc(tracks), ticks: fc(ticks), past: fc(past) };
}

export function distanceKm(a: [number, number], b: [number, number]): number {
  const toR = Math.PI / 180;
  const dLat = (b[1] - a[1]) * toR;
  const dLon = (b[0] - a[0]) * toR;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a[1] * toR) * Math.cos(b[1] * toR) * Math.sin(dLon / 2) ** 2;
  return 2 * 6371.0088 * Math.asin(Math.min(1, Math.sqrt(h)));
}

export interface IncomingStorm {
  minutes: number;
  peakDbz: number;
  speedMph: number;
  bearing: number;
}

/** The soonest tracked cell whose next-hour path passes within `radiusKm` of a place. */
export function incomingStorm(cells: StormCellInfo[], place: [number, number], radiusKm = 12): IncomingStorm | null {
  let best: IncomingStorm | null = null;
  for (const c of cells) {
    if (distanceKm([c.lon, c.lat], place) <= radiusKm) {
      if (!best || best.minutes > 0) best = { minutes: 0, peakDbz: c.peakDbz, speedMph: c.speedMph, bearing: c.bearing };
      continue;
    }
    if (!isTracked(c)) continue;
    for (let m = 5; m <= 60; m += 5) {
      if (distanceKm(advance(c.lon, c.lat, c.eastKmh, c.northKmh, m), place) <= radiusKm) {
        if (!best || m < best.minutes) best = { minutes: m, peakDbz: c.peakDbz, speedMph: c.speedMph, bearing: c.bearing };
        break;
      }
    }
  }
  return best;
}

export const compass = (deg: number) => ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][Math.round((((deg % 360) + 360) % 360) / 45) % 8];

export function hailLabel(peakDbz: number): string | null {
  if (peakDbz >= 60) return "Hail likely";
  if (peakDbz >= 55) return "Hail possible";
  return null;
}

/** "Hail-producing storm arriving in ~20 min" — shared wording for the Home card. */
export function describeIncoming(t: IncomingStorm): { headline: string; sub: string; severe: boolean } {
  const what = t.peakDbz >= 60 ? "Hail-producing storm" : t.peakDbz >= 50 ? "Heavy storm" : "Rain";
  return {
    headline: `${what} ${t.minutes === 0 ? "over you now" : `arriving in ~${t.minutes} min`}`,
    sub: `${Math.round(t.peakDbz)} dBZ · moving ${compass(t.bearing)} at ${Math.round(t.speedMph)} mph`,
    severe: t.peakDbz >= 55,
  };
}
