/**
 * Level II close-up rules shared by the web radar and the app: station choice, age and range
 * limits, the z8→9 fade, the six-sweep loop, MRMS alignment, beam height, provenance and the
 * viewport-view lifecycle. Design: docs/develop/design/level2-close-up.md. No imports: the app
 * compiles this file too.
 */

export const MAX_GROUND_RANGE_M = 150_000;
export const MAX_AGE_S = 600;
export const FADE_START_ZOOM = 8;
export const FADE_END_ZOOM = 9;
export const MIN_SOURCE_ZOOM = 8;
export const MAX_SOURCE_ZOOM = 13;
export const LOOP_FRAMES = 6;
export const UNDERLAY_MAX_AGE_S = 600;
export const REFRESH_MS = 15_000;
export const PENDING_LIMIT_MS = 30_000;
export const MAX_VIEW_TILES = 64;
export const MAX_VIEW_SPAN = 8;
export const DISABLED_RETRY_MS = 5 * 60_000;
const DEBOUNCE_MS = 250;
const EARTH_RADIUS_M = 6_371_000;
const TILE_TEMPLATE = "/level2/tiles/{station}/{path}/{palette}/{z}/{x}/{y}.png";
export const LEVEL2_PALETTES = ["classic", "muted", "vivid"] as const;
export type Level2Palette = (typeof LEVEL2_PALETTES)[number];

/** Every request goes to our own server; nothing here may name another host. */
const OWN_PATH = /^\/(api\/level2\/(sites|views(\/[0-9a-f]{32}(\/[0-9a-f]{16}\/manifest\.json)?)?)|level2\/tiles\/)/;
export const isOwnLevel2Path = (path: string): boolean => OWN_PATH.test(path);

// ---------- time and geometry ----------

const RFC3339 = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

/** Epoch ms of an RFC 3339 time, keeping sub-millisecond digits so 600.0005 s is not 600 s. */
export function parseTimeMs(text: unknown): number | null {
  if (typeof text !== "string") return null;
  const m = RFC3339.exec(text);
  if (!m) return null;
  const whole = Date.parse(`${m[1]}${m[3]}`);
  if (!Number.isFinite(whole)) return null;
  return whole + (m[2] ? Number(`0${m[2]}`) * 1000 : 0);
}

export function groundDistanceM(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const rad = Math.PI / 180;
  const a = Math.sin(((lat2 - lat1) * rad) / 2) ** 2 + Math.cos(lat1 * rad) * Math.cos(lat2 * rad) * Math.sin(((lon2 - lon1) * rad) / 2) ** 2;
  return EARTH_RADIUS_M * 2 * Math.asin(Math.sqrt(Math.min(1, Math.max(0, a))));
}

/** Exactly 150 km is inside; the server uses the same bound. */
export const inCoverage = (groundM: number): boolean => groundM <= MAX_GROUND_RANGE_M + Number.EPSILON * MAX_GROUND_RANGE_M;

/** Linear fade: MRMS only at z≤8, full Level II from z9. */
export const level2Opacity = (zoom: number): number => Math.min(1, Math.max(0, zoom - FADE_START_ZOOM));

/** One map pixel: Level II shows only inside 150 km and where its tile has an echo; else MRMS. */
export function pixelSource(zoom: number, groundM: number, alpha: number): "level2" | "mrms" {
  return level2Opacity(zoom) > 0 && inCoverage(groundM) && alpha > 0 ? "level2" : "mrms";
}

/** Beam centre above the radar on the 4/3 effective Earth (not above terrain). Null past the horizon. */
export function beamHeightM(groundM: number, elevationDeg: number): number | null {
  const k = (EARTH_RADIUS_M * 4) / 3;
  const d = groundM / k;
  const e = (elevationDeg * Math.PI) / 180;
  const den = Math.cos(e + d);
  if (!(den > 1e-6)) return null;
  const r = (k * Math.sin(d)) / den;
  return Math.sqrt(r * r + k * k + 2 * r * k * Math.sin(e)) - k;
}

// ---------- sites, health and station choice ----------

export interface Level2Health { state: string; eligible: boolean; low_sweep_end: string | null }
export interface Level2Site { id: string; name: string; lat: number; lon: number; enabled: boolean; health: Level2Health | null }

const isNum = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
const isText = (v: unknown): v is string => typeof v === "string" && v.length > 0;

/** Throws on shape drift so a client shows MRMS rather than guessing. */
export function parseSites(value: unknown): Level2Site[] {
  const v = value as { schema_version?: unknown; sites?: unknown } | null;
  if (v?.schema_version !== 1 || !Array.isArray(v.sites)) throw new Error("Level II sites do not match the contract");
  return v.sites.map((s: any) => {
    const h = s?.health;
    if (!isText(s?.id) || !isNum(s.lat) || !isNum(s.lon) || typeof s.enabled !== "boolean") throw new Error("Level II site drifted");
    return {
      id: s.id,
      name: typeof s.name === "string" ? s.name : s.id,
      lat: s.lat,
      lon: s.lon,
      enabled: s.enabled,
      health: h && typeof h === "object" ? { state: String(h.state), eligible: h.eligible === true, low_sweep_end: typeof h.low_sweep_end === "string" ? h.low_sweep_end : null } : null,
    };
  });
}

/** Seconds since the sweep ended; null for a missing or unparseable time. Negative means a future time. */
export function sweepAgeS(endIso: string | null | undefined, nowMs: number): number | null {
  const end = parseTimeMs(endIso);
  return end === null ? null : (nowMs - end) / 1000;
}

/** Eligible through exactly 600 s; a future end time is rejected. */
export function isFreshSweep(endIso: string | null | undefined, nowMs: number): boolean {
  const age = sweepAgeS(endIso, nowMs);
  return age !== null && age >= 0 && age <= MAX_AGE_S;
}

export interface Candidate { id: string; distanceM: number }

/** Nearest candidate inside 150 km; equal distances go to the lower station id. */
export function pickNearest(candidates: readonly Candidate[]): Candidate | null {
  const inside = candidates.filter((c) => inCoverage(c.distanceM));
  inside.sort((a, b) => a.distanceM - b.distanceM || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return inside[0] ?? null;
}

export type StationChoice = { ok: true; id: string; site: Level2Site; distanceM: number } | { ok: false; reason: string; enabled: boolean };

/** The nearest healthy station within range, judged on the client clock; else why none. */
export function selectStation(sites: readonly Level2Site[], centre: { lat: number; lon: number }, nowMs: number): StationChoice {
  const enabled = sites.filter((s) => s.enabled);
  if (enabled.length === 0) return { ok: false, reason: "off", enabled: false };
  const inRange = enabled.map((s) => ({ s, d: groundDistanceM(centre.lat, centre.lon, s.lat, s.lon) })).filter((x) => inCoverage(x.d));
  if (inRange.length === 0) return { ok: false, reason: "out_of_range", enabled: true };
  const healthy = inRange.filter((x) => x.s.health?.state === "healthy" && isFreshSweep(x.s.health.low_sweep_end, nowMs));
  const best = pickNearest(healthy.map((x) => ({ id: x.s.id, distanceM: x.d })));
  if (best) return { ok: true, id: best.id, site: enabled.find((s) => s.id === best.id)!, distanceM: best.distanceM };
  const nearest = [...inRange].sort((a, b) => a.d - b.d || (a.s.id < b.s.id ? -1 : 1))[0].s;
  const h = nearest.health;
  const reason = !h || h.state === "unknown" || h.state === "warming" || h.low_sweep_end === null ? "warming" : h.state === "future_clock" ? "future_clock" : "late";
  return { ok: false, reason, enabled: true };
}

// ---------- tiles wanted for a view ----------

export interface Viewport {
  centre: { lat: number; lon: number };
  zoom: number;
  /** West, south, east, north in degrees. */
  bounds: [number, number, number, number];
  palette: Level2Palette;
  /** Observed radar is the selected layer and no replay is showing. */
  active: boolean;
}

export const sourceZoom = (zoom: number): number => Math.min(MAX_SOURCE_ZOOM, Math.max(MIN_SOURCE_ZOOM, Math.floor(zoom)));

const tileX = (lon: number, z: number): number => ((lon + 180) / 360) * 2 ** z;
const tileY = (lat: number, z: number): number => {
  const r = (Math.max(-85, Math.min(85, lat)) * Math.PI) / 180;
  return ((1 - Math.log(Math.tan(r) + 1 / Math.cos(r)) / Math.PI) / 2) * 2 ** z;
};

interface Range { x0: number; x1: number; y0: number; y1: number }

const fitAxis = (lo: number, hi: number, centre: number, n: number): [number, number] => {
  if (hi - lo + 1 <= MAX_VIEW_SPAN) return [lo, hi];
  const start = Math.min(Math.max(0, n - MAX_VIEW_SPAN), Math.max(0, Math.floor(centre) - 3));
  return [start, start + MAX_VIEW_SPAN - 1];
};

/** The visible tiles at the source zoom, cut to the 8×8 the server accepts around the centre. */
export function coreRange(vp: Viewport): { zoom: number; range: Range } {
  const z = sourceZoom(vp.zoom);
  const n = 2 ** z;
  const [w, s, e, nn] = vp.bounds;
  const clamp = (v: number) => Math.min(n - 1, Math.max(0, Math.floor(v)));
  const cx = tileX(vp.centre.lon, z);
  const cy = tileY(vp.centre.lat, z);
  const [x0, x1] = fitAxis(clamp(tileX(w, z)), clamp(tileX(e, z)), cx, n);
  const [y0, y1] = fitAxis(clamp(tileY(nn, z)), clamp(tileY(s, z)), cy, n);
  return { zoom: z, range: { x0, x1, y0, y1 } };
}

const listTiles = (r: Range): [number, number][] => {
  const out: [number, number][] = [];
  for (let x = r.x0; x <= r.x1; x++) for (let y = r.y0; y <= r.y1; y++) out.push([x, y]);
  return out;
};

/** The exact XYZ set to ask for: the core plus a one-tile margin where the server limits allow. */
export function demandTiles(vp: Viewport): { zoom: number; tiles: [number, number][]; core: [number, number][] } {
  const { zoom, range } = coreRange(vp);
  const n = 2 ** zoom;
  const grow = (lo: number, hi: number, max: number): [number, number] => (hi - lo + 1 + 2 <= MAX_VIEW_SPAN ? [Math.max(0, lo - 1), Math.min(max, hi + 1)] : [lo, hi]);
  const [x0, x1] = grow(range.x0, range.x1, n - 1);
  const [y0, y1] = grow(range.y0, range.y1, n - 1);
  const tiles = listTiles({ x0, x1, y0, y1 });
  return { zoom, tiles: tiles.slice(0, MAX_VIEW_TILES), core: listTiles(range) };
}

// ---------- view status and manifest envelope ----------

export type ViewState = "pending" | "ready" | "fallback" | "expired";
export interface ViewStatus {
  state: ViewState;
  viewId: string | null;
  stationId: string | null;
  reason: string | null;
  pollAfterMs: number;
  manifestUrl: string | null;
  expiresAtMs: number | null;
}

const VIEW_ID = /^[0-9a-f]{32}$/;
const MANIFEST_URL = /^\/api\/level2\/views\/[0-9a-f]{32}\/[0-9a-f]{16}\/manifest\.json$/;

export function parseViewStatus(value: unknown): ViewStatus {
  const v = value as any;
  const state = v?.state;
  if (v?.schema_version !== 1 || !["pending", "ready", "fallback", "expired"].includes(state)) throw new Error("Level II view status drifted");
  const viewId = v.view_id == null ? null : String(v.view_id);
  if (viewId !== null && !VIEW_ID.test(viewId)) throw new Error("Level II view id drifted");
  if (state === "ready" && (viewId === null || typeof v.manifest_url !== "string" || !MANIFEST_URL.test(v.manifest_url) || !v.manifest_url.includes(`/${viewId}/`))) {
    throw new Error("Level II ready view has no own manifest URL");
  }
  const poll = isNum(v.poll_after_s) ? v.poll_after_s : 15;
  return {
    state,
    viewId,
    stationId: typeof v.station_id === "string" ? v.station_id : null,
    reason: typeof v.reason === "string" ? v.reason : null,
    pollAfterMs: Math.min(15, Math.max(2, poll)) * 1000,
    manifestUrl: state === "ready" ? v.manifest_url : null,
    expiresAtMs: parseTimeMs(v.expires_at),
  };
}

export interface Level2Frame {
  /** Sweep end in epoch seconds: the frame's time on every timeline. */
  time: number;
  timestamp: string;
  scanEndMs: number;
  scanStart: string | null;
  /** Tile path under the station: `<sweep>/<revision>`. */
  path: string;
  sweepId: string;
  volumeId: string | null;
  stationId: string;
  stationName: string;
  stationLat: number;
  stationLon: number;
  elevationDeg: number;
  antennaAltitudeMMsl: number | null;
  qcProfile: string | null;
  decoder: string | null;
  dualPol: { rho: string | null; zdr: string | null };
  sourceCount: number;
  samplerVersion: string | null;
  missingSectors: number;
}

export interface Level2View {
  viewId: string;
  revision: string;
  stationId: string;
  stationName: string;
  stationLat: number;
  stationLon: number;
  sourceZoom: number;
  palette: Level2Palette;
  requiredTiles: { z: number; x: number; y: number }[];
  tileTemplate: string;
  expiresAtMs: number;
  frames: Level2Frame[];
}

/** Validates the viewport envelope. Frames come back oldest first, at most six, never duplicated. */
export function parseEnvelope(value: unknown): Level2View {
  const e = value as any;
  const bad = (why: string): never => {
    throw new Error(`Level II manifest ${why}`);
  };
  if (e?.schema_version !== 1) bad("has the wrong schema version");
  if (!VIEW_ID.test(String(e.view_id)) || !/^[0-9a-f]{16}$/.test(String(e.revision))) bad("has a bad id");
  const station = e.station;
  if (!isText(station?.id) || !isNum(station.lat) || !isNum(station.lon)) bad("has no station");
  const zoom = e.source_zoom;
  if (!Number.isInteger(zoom) || zoom < MIN_SOURCE_ZOOM || zoom > MAX_SOURCE_ZOOM) bad("has a bad source zoom");
  if (e.tile_url_template !== TILE_TEMPLATE) bad("has a foreign tile template");
  const palette = Array.isArray(e.palettes) && e.palettes.length === 1 ? e.palettes[0] : null;
  if (!(LEVEL2_PALETTES as readonly string[]).includes(palette)) bad("has a bad palette");
  const required = e.required_tiles;
  if (!Array.isArray(required) || required.length === 0 || required.length > MAX_VIEW_TILES || !required.every((t: any) => t?.z === zoom && Number.isInteger(t.x) && Number.isInteger(t.y))) bad("has bad required tiles");
  const expiresAtMs = parseTimeMs(e.expires_at);
  if (expiresAtMs === null) bad("has no expiry");
  const layer = e.manifest?.layers?.[`level2-ref-${station.id}`];
  const raw = layer?.frames;
  if (!Array.isArray(raw) || raw.length === 0 || raw.length > LOOP_FRAMES) bad("has no frames or too many");
  const frames: Level2Frame[] = raw.map((f: any) => {
    const end = parseTimeMs(f?.scan_end);
    if (end === null || f.timestamp !== f.scan_end || f.station_id !== station.id) bad("has a bad frame time");
    if (!/^[A-Za-z0-9_-]{1,128}\/[A-Za-z0-9_-]{1,128}$/.test(String(f.path)) || !isNum(f.elevation_deg)) bad("has a bad frame path");
    const dp = f.dual_pol ?? {};
    return {
      time: Math.floor((end as number) / 1000),
      timestamp: f.timestamp,
      scanEndMs: end as number,
      scanStart: typeof f.scan_start === "string" ? f.scan_start : null,
      path: f.path,
      sweepId: String(f.sweep_id ?? ""),
      volumeId: typeof f.volume_id === "string" ? f.volume_id : null,
      stationId: station.id,
      stationName: typeof f.station_name === "string" ? f.station_name : String(station.name ?? station.id),
      stationLat: isNum(f.station_lat) ? f.station_lat : station.lat,
      stationLon: isNum(f.station_lon) ? f.station_lon : station.lon,
      elevationDeg: f.elevation_deg,
      antennaAltitudeMMsl: isNum(f.antenna_altitude_m_msl) ? f.antenna_altitude_m_msl : null,
      qcProfile: typeof f.qc_profile === "string" ? f.qc_profile : null,
      decoder: typeof f.decoder === "string" ? f.decoder : f.decoder && typeof f.decoder === "object" ? JSON.stringify(f.decoder) : null,
      dualPol: { rho: typeof dp.rho === "string" ? dp.rho : null, zdr: typeof dp.zdr === "string" ? dp.zdr : null },
      sourceCount: Array.isArray(f.sources) ? f.sources.length : f.sources && typeof f.sources === "object" ? Object.keys(f.sources).length : 0,
      samplerVersion: typeof f.sampler_version === "string" ? f.sampler_version : null,
      missingSectors: Array.isArray(f.missing_sectors_deg) ? f.missing_sectors_deg.length : 0,
    };
  });
  frames.sort((a, b) => a.scanEndMs - b.scanEndMs);
  for (let i = 1; i < frames.length; i++) if (frames[i].scanEndMs === frames[i - 1].scanEndMs) bad("repeats a frame time");
  return {
    viewId: e.view_id,
    revision: e.revision,
    stationId: station.id,
    stationName: typeof station.name === "string" ? station.name : station.id,
    stationLat: station.lat,
    stationLon: station.lon,
    sourceZoom: zoom,
    palette: palette as Level2Palette,
    requiredTiles: required,
    tileTemplate: e.tile_url_template,
    expiresAtMs: expiresAtMs as number,
    frames,
  };
}

/** Why a held view can no longer be shown, judged on the client clock; null while it may be. */
export function viewProblem(view: Level2View, nowMs: number): "expired" | "late" | null {
  if (nowMs >= view.expiresAtMs) return "expired";
  const newest = view.frames[view.frames.length - 1];
  return isFreshSweep(newest.timestamp, nowMs) ? null : "late";
}

/** True when every wanted core tile is one the view promises. */
export function viewCovers(view: Level2View, zoom: number, core: readonly [number, number][]): boolean {
  if (view.sourceZoom !== zoom) return false;
  const have = new Set(view.requiredTiles.map((t) => `${t.x},${t.y}`));
  return core.every(([x, y]) => have.has(`${x},${y}`));
}

/** Tile URL for one frame with {z}/{x}/{y} left for the map. Own server only. */
export function level2TileUrl(view: Level2View, frame: Level2Frame): string {
  return view.tileTemplate
    .replace("{station}", encodeURIComponent(frame.stationId))
    .replace("{path}", frame.path.split("/").map(encodeURIComponent).join("/"))
    .replace("{palette}", encodeURIComponent(view.palette));
}

/** The tile bounds a source can cover, as [west, south, east, north], so no other tile is asked for. */
export function viewBounds(view: Level2View): [number, number, number, number] {
  const n = 2 ** view.sourceZoom;
  const xs = view.requiredTiles.map((t) => t.x);
  const ys = view.requiredTiles.map((t) => t.y);
  const lon = (x: number) => (x / n) * 360 - 180;
  const lat = (y: number) => (Math.atan(Math.sinh(Math.PI * (1 - (2 * y) / n))) * 180) / Math.PI;
  return [lon(Math.min(...xs)), lat(Math.max(...ys) + 1), lon(Math.max(...xs) + 1), lat(Math.min(...ys))];
}

// ---------- the loop and the MRMS underlay ----------

export interface TimedFrame { time: number }

/** Index of the newest observation at or before the scan, at most 10 min older; -1 is an explicit gap. */
export function underlayIndex(scanSec: number, mrms: readonly TimedFrame[]): number {
  let best = -1;
  for (let i = 0; i < mrms.length; i++) if (mrms[i].time <= scanSec && (best < 0 || mrms[i].time >= mrms[best].time)) best = i;
  return best >= 0 && scanSec - mrms[best].time <= UNDERLAY_MAX_AGE_S ? best : -1;
}

export interface CloseUpFrame { time: number; level2: Level2Frame; mrmsIndex: number; gap: boolean }

/** The last six completed low sweeps in acquisition order, each with its MRMS underlay or a gap. No thinning. */
export function closeUpFrames(view: Level2View, mrms: readonly TimedFrame[]): CloseUpFrame[] {
  return view.frames.slice(-LOOP_FRAMES).map((level2) => {
    const mrmsIndex = underlayIndex(level2.time, mrms);
    return { time: level2.time, level2, mrmsIndex, gap: mrmsIndex < 0 };
  });
}

// ---------- wording ----------

const KM_PER_MI = 1.609344;
const FT_PER_M = 3.28084;

export function formatGround(m: number): string {
  return `${Math.round(m / 1000)} km (${Math.round(m / 1000 / KM_PER_MI)} mi)`;
}

export function formatHeight(m: number | null): string {
  if (m === null) return "unknown";
  const ft = Math.round((m * FT_PER_M) / 100) * 100;
  return m < 1000 ? `${Math.round(m / 10) * 10} m (${ft.toLocaleString("en-US")} ft)` : `${(m / 1000).toFixed(1)} km (${ft.toLocaleString("en-US")} ft)`;
}

const clockFormat = (timeZone?: string) => new Intl.DateTimeFormat("en-US", { timeZone, hour: "numeric", minute: "2-digit", timeZoneName: "short" });
const fullFormat = (timeZone?: string) => new Intl.DateTimeFormat("en-US", { timeZone, weekday: "short", year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit", timeZoneName: "short" });

export const scanClock = (ms: number, timeZone?: string): string => clockFormat(timeZone).format(new Date(ms));
export const scanFull = (ms: number, timeZone?: string): string => fullFormat(timeZone).format(new Date(ms));
const utcClock = (ms: number): string => `${new Date(ms).toISOString().slice(0, 16).replace("T", " ")} UTC`;

export interface Provenance {
  /** One line beside the timeline / legend. */
  headline: string;
  groundRange: string;
  beamHeight: string;
  scanClock: string;
  /** Rows for the details panel. */
  rows: { label: string; value: string }[];
}

/** Provenance of the selected frame at the current map centre: station, scan, tilt, range, beam height. */
export function provenance(frame: Level2Frame, centre: { lat: number; lon: number }, timeZone?: string): Provenance {
  const ground = groundDistanceM(centre.lat, centre.lon, frame.stationLat, frame.stationLon);
  const height = beamHeightM(ground, frame.elevationDeg);
  const clock = scanClock(frame.scanEndMs, timeZone);
  const tilt = `${frame.elevationDeg.toFixed(1)}°`;
  const beam = `~${formatHeight(height)}`;
  const dual = frame.dualPol.rho || frame.dualPol.zdr ? `RHO ${frame.dualPol.rho ?? "unknown"}, ZDR ${frame.dualPol.zdr ?? "unknown"}` : "not reported";
  return {
    headline: `Single-site observed · ${frame.stationId} · scan ${clock} · ${tilt} · beam ${beam} above radar at centre`,
    groundRange: `${formatGround(ground)} from radar at centre`,
    beamHeight: beam,
    scanClock: clock,
    rows: [
      { label: "Station", value: `${frame.stationId} ${frame.stationName}` },
      { label: "Scan", value: `${scanFull(frame.scanEndMs, timeZone)} · ${utcClock(frame.scanEndMs)}` },
      { label: "Tilt", value: `${tilt} actual elevation of this sweep` },
      { label: "Ground range", value: `${formatGround(ground)} from the radar at the map centre; Level II stops at 150 km` },
      { label: "Beam height", value: `${beam} above the radar at centre, estimated on a 4/3 effective Earth; not height above terrain${frame.antennaAltitudeMMsl === null ? "" : `; antenna ${Math.round(frame.antennaAltitudeMMsl)} m above sea level`}` },
      { label: "Quality control", value: `${frame.qcProfile ?? "unknown profile"}; dual-pol ${dual}${frame.missingSectors > 0 ? `; ${frame.missingSectors} missing sector${frame.missingSectors === 1 ? "" : "s"}` : ""}` },
      { label: "Source", value: `NOAA NEXRAD Level II, ${frame.sourceCount} source object${frame.sourceCount === 1 ? "" : "s"}, served by this radar server${frame.decoder ? `; ${frame.decoder}` : ""}` },
      { label: "Caution", value: "Reflectivity aloft at one site, not surface rainfall. Range-gate spacing is not uniform ground resolution." },
    ],
  };
}

const REASONS: Record<string, string> = {
  warming: "station warming up",
  late: "newest sweep older than 10 min",
  stale: "newest sweep older than 10 min",
  expired: "view expired",
  view_expired: "view expired",
  out_of_range: "no station within 150 km",
  saturated: "close-up capacity full",
  source_unreachable: "radar feed unreachable",
  future_clock: "clock mismatch",
  pending_timeout: "took too long to prepare",
  tile_missing: "a tile is missing",
  unavailable: "not available",
  rate_limited: "too many requests",
  unreachable: "server unreachable",
  changed: "view changed",
};

/** "MRMS · Level II unavailable" with the short reason; the base layer stays on screen. */
export function fallbackCopy(reason: string | null): { title: string; reason: string } {
  return { title: "MRMS · Level II unavailable", reason: (reason && REASONS[reason]) || "close-up not available here" };
}

// ---------- the view lifecycle ----------

export type Level2Phase = "off" | "idle" | "warming" | "ready" | "fallback";

export interface Level2Snapshot {
  phase: Level2Phase;
  reason: string | null;
  /** Only while the view may be shown: fresh sweep, unexpired, current palette, station and zoom. */
  view: Level2View | null;
  opacity: number;
  /** Show "MRMS · Level II unavailable" now. */
  unavailable: boolean;
  /** Changes whenever what the map should draw changes. */
  key: string;
}

export interface Level2Deps {
  request(path: string, init: { method: "GET" | "POST"; body?: string; signal: AbortSignal }): Promise<{ status: number; json: unknown }>;
  now?: () => number;
}

interface Wanted { station: string; zoom: number; palette: Level2Palette; tiles: [number, number][]; core: [number, number][] }

const bodyFor = (vp: Viewport, w: Wanted) => JSON.stringify({ centre: { lat: vp.centre.lat, lon: vp.centre.lon }, zoom: w.zoom, tiles: w.tiles, palette: w.palette });

/**
 * One close-up view per client: picks the station, asks for the exact tile set, polls until ready,
 * refreshes every 15 s, and drops to MRMS the moment anything it relied on changes or expires.
 * Platform glue supplies `request` and listens with `subscribe`.
 */
export class Level2Session {
  private deps: Required<Level2Deps>;
  private listeners = new Set<() => void>();
  private vp: Viewport | null = null;
  private sites: Level2Site[] | null = null;
  private sitesAt = 0;
  private disabledUntil = 0;
  private view: Level2View | null = null;
  private lastRevision: string | null = null;
  private scope: { body: string; station: string; zoom: number; palette: Level2Palette } | null = null;
  private pending: { key: string; since: number } | null = null;
  private reason: string | null = null;
  private token = 0;
  private ctl: AbortController | null = null;
  private tick: ReturnType<typeof setTimeout> | null = null;
  private debounce: ReturnType<typeof setTimeout> | null = null;
  private expiry: ReturnType<typeof setTimeout> | null = null;
  private retryAfter = 0;
  private stopped = false;

  constructor(deps: Level2Deps) {
    this.deps = { now: () => Date.now(), ...deps };
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify() {
    for (const fn of [...this.listeners]) fn();
  }

  private wants(): boolean {
    const vp = this.vp;
    return !this.stopped && !!vp && vp.active && vp.zoom >= FADE_START_ZOOM && this.deps.now() >= this.disabledUntil;
  }

  snapshot(): Level2Snapshot {
    const now = this.deps.now();
    const vp = this.vp;
    const enabled = !!this.sites?.some((s) => s.enabled);
    const problem = this.view ? viewProblem(this.view, now) : null;
    const usable = this.view && !problem && vp && this.view.palette === vp.palette && this.view.sourceZoom === sourceZoom(vp.zoom) ? this.view : null;
    const showing = !!vp && this.wants();
    const opacity = usable && vp ? level2Opacity(vp.zoom) : 0;
    const phase: Level2Phase = !showing ? (now < this.disabledUntil ? "off" : "idle") : usable ? "ready" : this.pending ? "warming" : enabled || this.reason ? "fallback" : "idle";
    const reason = usable ? null : problem ?? this.reason ?? (this.pending ? "warming" : null);
    const unavailable = showing && enabled && !usable && !!vp && vp.zoom > FADE_START_ZOOM && phase !== "idle";
    return { phase, reason, view: usable, opacity, unavailable, key: `${phase}|${usable ? `${usable.viewId}.${usable.revision}` : ""}|${opacity.toFixed(3)}|${unavailable}|${reason ?? ""}` };
  }

  /** New camera, palette or layer state. Cheap: changes only the fade unless the wanted view changed. */
  update(vp: Viewport): void {
    this.vp = vp;
    this.reconcile();
  }

  /** A tile the view promised failed to load: show MRMS now and ask again at the next refresh. */
  reportTileFailure(): void {
    if (!this.view) return;
    this.dropView("tile_missing");
    this.retryAfter = this.deps.now() + REFRESH_MS;
    this.notify();
  }

  stop(): void {
    this.stopped = true;
    this.cancel();
    for (const t of [this.tick, this.debounce, this.expiry]) if (t) clearTimeout(t);
    this.tick = this.debounce = this.expiry = null;
    this.listeners.clear();
  }

  private cancel() {
    this.token++;
    this.ctl?.abort();
    this.ctl = null;
    this.pending = null;
  }

  private dropView(reason: string | null) {
    this.cancel();
    this.view = null;
    this.lastRevision = null;
    this.scope = null;
    this.reason = reason;
    if (this.expiry) clearTimeout(this.expiry);
    this.expiry = null;
  }

  private async call(path: string, method: "GET" | "POST", body: string | undefined, ctl: AbortController) {
    if (!isOwnLevel2Path(path)) throw new Error("refusing a request outside this server");
    return this.deps.request(path, { method, body, signal: ctl.signal });
  }

  private armTick() {
    if (this.tick || this.stopped) return;
    this.tick = setTimeout(() => {
      this.tick = null;
      void this.refresh();
    }, REFRESH_MS);
  }

  private armExpiry() {
    if (this.expiry) clearTimeout(this.expiry);
    this.expiry = null;
    const v = this.view;
    if (!v) return;
    const newest = v.frames[v.frames.length - 1];
    const at = Math.min(v.expiresAtMs, newest.scanEndMs + MAX_AGE_S * 1000 + 1);
    this.expiry = setTimeout(() => {
      this.expiry = null;
      const problem = this.view ? viewProblem(this.view, this.deps.now()) : null;
      if (problem) {
        this.dropView(problem);
        this.notify();
        this.reconcile();
      }
    }, Math.max(0, at - this.deps.now()));
  }

  private async refresh() {
    if (!this.wants()) return;
    this.armTick();
    await this.loadSites();
    this.reconcile(true);
  }

  private async loadSites() {
    const ctl = new AbortController();
    try {
      const res = await this.call("/api/level2/sites", "GET", undefined, ctl);
      if (res.status === 404) {
        this.disabledUntil = this.deps.now() + DISABLED_RETRY_MS;
        this.sites = null;
        this.dropView(null);
        this.notify();
        return;
      }
      if (res.status !== 200) throw new Error(`sites ${res.status}`);
      this.sites = parseSites(res.json);
      this.sitesAt = this.deps.now();
    } catch {
      // Keep the last list only while it is within two refreshes; after that Level II is unavailable.
      if (this.sites && this.deps.now() - this.sitesAt > 2 * REFRESH_MS) {
        this.sites = null;
        this.dropView("unreachable");
      }
    }
  }

  private wanted(): Wanted | null {
    const vp = this.vp;
    if (!vp || !this.sites) return null;
    const pick = selectStation(this.sites, vp.centre, this.deps.now());
    if (!pick.ok) {
      if (this.view) this.dropView(pick.reason);
      this.reason = pick.reason === "off" ? null : pick.reason;
      return null;
    }
    const d = demandTiles(vp);
    return { station: pick.id, zoom: d.zoom, palette: vp.palette, tiles: d.tiles, core: d.core };
  }

  private reconcile(fromTick = false) {
    if (!this.wants()) {
      if (this.view || this.pending) this.dropView(null);
      this.notify();
      return;
    }
    this.armTick();
    if (!this.sites) {
      if (!this.pending) void this.loadSites().then(() => this.reconcile());
      this.notify();
      return;
    }
    const w = this.wanted();
    if (!w) {
      this.notify();
      return;
    }
    // A different station, zoom or palette is a different promise: MRMS now, new view when ready.
    const stale = (this.view && (this.view.stationId !== w.station || this.view.sourceZoom !== w.zoom || this.view.palette !== w.palette)) || (this.scope && (this.scope.station !== w.station || this.scope.zoom !== w.zoom || this.scope.palette !== w.palette));
    if (stale) this.dropView("changed");
    const covered = this.view && viewCovers(this.view, w.zoom, w.core) && !viewProblem(this.view, this.deps.now());
    if (covered && !fromTick) {
      this.notify();
      return;
    }
    if (this.deps.now() < this.retryAfter && !this.view) {
      this.notify();
      return;
    }
    const key = `${w.station}|${w.zoom}|${w.palette}|${w.tiles.map((t) => t.join(",")).join(";")}`;
    if (this.pending?.key === key && !fromTick) {
      this.notify();
      return;
    }
    if (this.debounce) clearTimeout(this.debounce);
    this.debounce = setTimeout(() => {
      this.debounce = null;
      void this.request(w, key, covered ? this.scope?.body : undefined);
    }, fromTick || covered ? 0 : DEBOUNCE_MS);
    this.notify();
  }

  private async request(w: Wanted, key: string, renewBody?: string) {
    const vp = this.vp;
    if (!vp || !this.wants()) return;
    const renewing = renewBody !== undefined;
    if (!renewing) this.cancel();
    const token = renewing ? this.token : ++this.token;
    const ctl = renewing ? new AbortController() : (this.ctl = new AbortController());
    if (!renewing) this.pending = { key, since: this.deps.now() };
    const body = renewBody ?? bodyFor(vp, w);
    const live = () => token === this.token && !this.stopped;
    try {
      let res = await this.call("/api/level2/views", "POST", body, ctl);
      if (!live()) return;
      if (res.status === 429) throw new Error("rate_limited");
      if (res.status >= 500 || res.status === 404) throw new Error("unreachable");
      if (res.status === 422) {
        this.fail(String((res.json as { reason?: string })?.reason ?? "unavailable"), w);
        return;
      }
      let status = parseViewStatus(res.json);
      const since = this.pending?.since ?? this.deps.now();
      while (status.state === "pending") {
        if (this.deps.now() - since > PENDING_LIMIT_MS) {
          this.fail("pending_timeout", w);
          return;
        }
        await new Promise((r) => setTimeout(r, status.pollAfterMs));
        if (!live()) return;
        res = await this.call(`/api/level2/views/${status.viewId}`, "GET", undefined, ctl);
        if (!live()) return;
        if (res.status !== 200) throw new Error("unreachable");
        status = parseViewStatus(res.json);
      }
      if (status.state !== "ready") {
        this.fail(status.reason ?? "unavailable", w);
        return;
      }
      if (status.stationId && status.stationId !== w.station) {
        this.fail("changed", w);
        return;
      }
      if (this.view && this.lastRevision && status.manifestUrl?.includes(`/${this.lastRevision}/`) && this.view.viewId === status.viewId) {
        if (status.expiresAtMs) this.view = { ...this.view, expiresAtMs: Math.max(this.view.expiresAtMs, status.expiresAtMs) };
        this.pending = null;
        this.armExpiry();
        this.notify();
        return;
      }
      const m = await this.call(status.manifestUrl as string, "GET", undefined, ctl);
      if (!live()) return;
      if (m.status !== 200) throw new Error(m.status === 410 ? "expired" : "unreachable");
      const view = parseEnvelope(m.json);
      if (view.stationId !== w.station || view.palette !== w.palette || view.sourceZoom !== w.zoom) {
        this.fail("changed", w);
        return;
      }
      if (viewProblem(view, this.deps.now())) {
        this.fail("late", w);
        return;
      }
      this.view = status.expiresAtMs ? { ...view, expiresAtMs: Math.max(view.expiresAtMs, status.expiresAtMs) } : view;
      this.lastRevision = view.revision;
      this.scope = { body, station: w.station, zoom: w.zoom, palette: w.palette };
      this.pending = null;
      this.reason = null;
      this.armExpiry();
      this.notify();
    } catch (err) {
      if (!live() || (err as Error)?.name === "AbortError") return;
      const msg = (err as Error)?.message;
      this.fail(msg === "rate_limited" || msg === "expired" ? msg : "unreachable", w);
    }
  }

  private fail(reason: string, w: Wanted) {
    this.pending = null;
    this.reason = reason;
    // A held ready view that merely failed to refresh stays only while its own clock allows it.
    if (!this.view || reason === "changed") this.scope = this.scope && this.scope.station === w.station ? this.scope : null;
    if (this.view && viewProblem(this.view, this.deps.now())) this.dropView(reason);
    this.retryAfter = this.deps.now() + REFRESH_MS;
    this.notify();
  }
}
