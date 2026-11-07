/**
 * Tracked storm cells: the one place the track rule, the hour-ahead cone, the
 * "incoming storm" search and the "storm reaches you" notice are defined. The
 * web radar and the app both import this file (the app through Metro's
 * watchFolders), so the two always draw and warn the same way.
 *
 * Dependency-free and pure. It runs on the JS thread and feeds MapLibre
 * GeoJSON sources; do not call it from a Reanimated worklet or a Skia frame
 * callback (it allocates).
 */

export const TRACK_MINUTES = [15, 30, 45, 60];
export const MIN_TRACK_CONFIDENCE = 0.4;

export interface StormCell {
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

export type LonLat = [number, number];

export function advance(lon: number, lat: number, eastKmh: number, northKmh: number, minutes: number): LonLat {
  const h = minutes / 60;
  return [lon + (eastKmh * h) / Math.max(1e-6, 111.32 * Math.cos((lat * Math.PI) / 180)), lat + (northKmh * h) / 110.574];
}

/** History-backed fits only: one-step vectors from older servers were noise. */
export const isTracked = (c: StormCell) =>
  c.history.length >= 3 && c.confidence >= MIN_TRACK_CONFIDENCE && c.speedMph >= 3 && c.speedMph <= 90;

const R_EARTH_KM = 6371.0088;
export function distanceKm(a: LonLat, b: LonLat): number {
  const toR = Math.PI / 180;
  const dLat = (b[1] - a[1]) * toR;
  const dLon = (b[0] - a[0]) * toR;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a[1] * toR) * Math.cos(b[1] * toR) * Math.sin(dLon / 2) ** 2;
  return 2 * R_EARTH_KM * Math.asin(Math.min(1, Math.sqrt(h)));
}

// ---------- incoming storm (Home card, web panel) ----------

export interface IncomingStorm {
  /** The cell that produced this result, so a notice can fire once per cell. */
  id: number;
  minutes: number;
  peakDbz: number;
  speedMph: number;
  bearing: number;
}

/** The soonest tracked cell whose next-hour path passes within `radiusKm` of a place. */
export function incomingStorm(cells: StormCell[], place: LonLat, radiusKm = 12): IncomingStorm | null {
  let best: IncomingStorm | null = null;
  for (const c of cells) {
    if (distanceKm([c.lon, c.lat], place) <= radiusKm) {
      if (!best || best.minutes > 0) best = { id: c.id, minutes: 0, peakDbz: c.peakDbz, speedMph: c.speedMph, bearing: c.bearing };
      continue;
    }
    if (!isTracked(c)) continue;
    for (let m = 5; m <= 60; m += 5) {
      if (distanceKm(advance(c.lon, c.lat, c.eastKmh, c.northKmh, m), place) <= radiusKm) {
        if (!best || m < best.minutes) best = { id: c.id, minutes: m, peakDbz: c.peakDbz, speedMph: c.speedMph, bearing: c.bearing };
        break;
      }
    }
  }
  return best;
}

// ---------- 60-minute cone ----------

export const CONE_MINUTES = 60;
/** Half-width of the cone either side of the cell's heading. */
export const CONE_HALF_ANGLE_DEG = 15;
const CONE_ARC_STEPS = 6;

/**
 * The area a tracked cell may cover in the next hour: a wedge from the cell,
 * +-15 degrees of its heading, as far as it travels in 60 minutes. Closed
 * ring of [lon, lat], or null when the cell is not tracked. The centre line
 * ends exactly where the 60-minute tick of the track does.
 */
export function stormCone(c: StormCell): LonLat[] | null {
  if (!isTracked(c)) return null;
  const speedKmh = Math.hypot(c.eastKmh, c.northKmh);
  if (speedKmh <= 0) return null;
  const heading = Math.atan2(c.eastKmh, c.northKmh);
  const half = (CONE_HALF_ANGLE_DEG * Math.PI) / 180;
  const ring: LonLat[] = [[c.lon, c.lat]];
  for (let i = 0; i <= CONE_ARC_STEPS; i++) {
    const a = heading - half + (2 * half * i) / CONE_ARC_STEPS;
    ring.push(advance(c.lon, c.lat, speedKmh * Math.sin(a), speedKmh * Math.cos(a), CONE_MINUTES));
  }
  ring.push([c.lon, c.lat]);
  return ring;
}

export interface ConeFeature {
  type: "Feature";
  properties: { id: number; severe: boolean };
  geometry: { type: "Polygon"; coordinates: LonLat[][] };
}

export function stormConeFeatures(cells: StormCell[]): ConeFeature[] {
  const out: ConeFeature[] = [];
  for (const c of cells) {
    const ring = stormCone(c);
    if (ring) out.push({ type: "Feature", properties: { id: c.id, severe: c.peakDbz >= 55 }, geometry: { type: "Polygon", coordinates: [ring] } });
  }
  return out;
}

// ---------- stale fade ----------

/** A track is only meaningful near the radar scan it was computed from. */
export const STALE_TRACK_MIN = 10;
export const STALE_OPACITY = 0.25;

/** Epoch seconds of the scan the `/api/storms` document was built from, or null. */
export function stormTrackTimeSec(doc: { timestamp?: unknown } | null | undefined): number | null {
  const t = typeof doc?.timestamp === "string" ? Date.parse(doc.timestamp) : NaN;
  return Number.isFinite(t) ? t / 1000 : null;
}

/** 1 at the live frame, STALE_OPACITY when the frame on screen is more than 10 min from the track time. */
export function stormLayerOpacity(frameSec: number | null | undefined, trackSec: number | null | undefined): number {
  if (frameSec == null || trackSec == null) return 1;
  return Math.abs(frameSec - trackSec) > STALE_TRACK_MIN * 60 ? STALE_OPACITY : 1;
}

// ---------- "storm reaches you" notice ----------

export const APPROACH_RADIUS_KM = 10;
export const APPROACH_HORIZON_MIN = 30;
export const APPROACH_MIN_DBZ = 55;
/** A cell id is remembered this long; ids restart when the server does. */
export const APPROACH_SEEN_TTL_MS = 6 * 3_600_000;

/**
 * What makes a tracked cell worth a notice. 55 dBZ is the only trigger today.
 * Hail and rotation (#256) plug in here: add a predicate; nothing else changes.
 */
export type ApproachTrigger = (c: StormCell) => boolean;
export const APPROACH_TRIGGERS: ApproachTrigger[] = [(c) => c.peakDbz >= APPROACH_MIN_DBZ];

export interface StormApproach {
  id: number;
  /** Whole minutes until the cell first comes within the radius; 0 when it already is. */
  minutes: number;
  peakDbz: number;
  speedMph: number;
  bearing: number;
}

/** Tracked cells meeting a trigger that come within the radius of `place` in the next 30 minutes, soonest first. */
export function stormApproaches(cells: StormCell[], place: LonLat): StormApproach[] {
  const out: StormApproach[] = [];
  for (const c of cells) {
    if (!isTracked(c) || !APPROACH_TRIGGERS.some((t) => t(c))) continue;
    for (let m = 0; m <= APPROACH_HORIZON_MIN; m++) {
      const at = m === 0 ? ([c.lon, c.lat] as LonLat) : advance(c.lon, c.lat, c.eastKmh, c.northKmh, m);
      if (distanceKm(at, place) <= APPROACH_RADIUS_KM) {
        out.push({ id: c.id, minutes: m, peakDbz: c.peakDbz, speedMph: c.speedMph, bearing: c.bearing });
        break;
      }
    }
  }
  return out.sort((a, b) => a.minutes - b.minutes);
}

/** cell id -> epoch ms it was announced. */
export type SeenApproaches = Record<string, number>;

/**
 * Drops cells already announced, and returns the updated memory (expired
 * entries pruned, the fresh ones added). Fire once per cell id, not per poll.
 */
export function takeNewApproaches(
  approaches: StormApproach[],
  seen: SeenApproaches,
  now: number,
): { fresh: StormApproach[]; seen: SeenApproaches } {
  const next: SeenApproaches = {};
  for (const [id, at] of Object.entries(seen)) if (now - at < APPROACH_SEEN_TTL_MS) next[id] = at;
  const fresh = approaches.filter((a) => !(String(a.id) in next));
  for (const a of fresh) next[String(a.id)] = now;
  return { fresh, seen: next };
}

export function parseSeenApproaches(raw: string | null | undefined): SeenApproaches {
  try {
    const v = JSON.parse(raw || "{}") as unknown;
    if (!v || typeof v !== "object" || Array.isArray(v)) return {};
    return Object.fromEntries(Object.entries(v).filter(([, at]) => typeof at === "number" && Number.isFinite(at))) as SeenApproaches;
  } catch {
    return {};
  }
}

export const NWS_DISCLAIMER = "Radar estimate, not an NWS warning. Follow NWS warnings for official alerts.";

export function approachCopy(a: StormApproach): { title: string; body: string } {
  const when = a.minutes <= 1 ? "now" : `in ~${a.minutes} min`;
  return {
    title: a.minutes <= 1 ? "Strong storm at your location" : `Strong storm may reach you ${when}`,
    body: `A ${Math.round(a.peakDbz)} dBZ cell is on course to pass within ${APPROACH_RADIUS_KM} km. ${NWS_DISCLAIMER}`,
  };
}
