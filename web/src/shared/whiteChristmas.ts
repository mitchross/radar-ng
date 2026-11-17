/**
 * White Christmas chance: the share of 1991-2020 years with at least 1 inch of
 * snow on the ground on Dec 25 at the nearest NCEI normals station. Our server
 * embeds the station table (GET /api/climate/white-christmas); this module is
 * the shared card logic for the web panel and the app. No interpolation.
 */

export const WHITE_CHRISTMAS_STATIONS_PATH = "/api/climate/white-christmas";
/** Farther than this, the nearest station says little about the place. */
export const WHITE_CHRISTMAS_MAX_MI = 60;
export const WHITE_CHRISTMAS_CREDIT = "NOAA NCEI 1991–2020 normals";

export interface WhiteChristmasStation {
  id: string;
  name: string;
  lat: number;
  lon: number;
  /** Percent of years, 0-100. */
  pct: number;
  years: number;
  distance_mi: number;
}

export interface WhiteChristmasCard {
  pctLabel: string;
  headline: string;
  detail: string;
  sentence: string;
}

/** Nearest-station URL; coordinates are rounded so a full fix never leaves the device. */
export function whiteChristmasPath(lat: number, lon: number): string {
  return `${WHITE_CHRISTMAS_STATIONS_PATH}?lat=${lat.toFixed(2)}&lon=${lon.toFixed(2)}`;
}

/** Nov 15 to Dec 25 inclusive, on the place's local date (`YYYY-MM-DD`). */
export function inWhiteChristmasSeason(dateKey: string): boolean {
  const md = dateKey.slice(5, 10);
  return md >= "11-15" && md <= "12-25";
}

/** The `station` of a nearest-station response, or null when it is missing or malformed. */
export function parseNearestStation(body: unknown): WhiteChristmasStation | null {
  const s = (body as { station?: Record<string, unknown> } | null)?.station;
  if (!s || typeof s !== "object") return null;
  const num = (k: string) => (typeof s[k] === "number" && Number.isFinite(s[k]) ? (s[k] as number) : null);
  const [lat, lon, pct, dist] = [num("lat"), num("lon"), num("pct"), num("distance_mi")];
  if (typeof s.name !== "string" || lat == null || lon == null || pct == null || dist == null) return null;
  return { id: String(s.id ?? ""), name: s.name, lat, lon, pct, years: num("years") ?? 0, distance_mi: dist };
}

const KEEP_UPPER = new Set(["US", "NWS", "WSO", "WFO", "FAA", "TV", "AFB", "NAS"]);
const WORDS: Record<string, string> = { AP: "Airport", INTL: "Intl", MUNI: "Muni", RGNL: "Regional", CNTRL: "Central", PK: "Park", TWR: "Tower" };

/** "GRAND RAPIDS INTL AP, MI US" -> "Grand Rapids Intl Airport, MI". */
export function stationLabel(name: string): string {
  const [place, region = ""] = name.split(",").map((p) => p.trim());
  const state = region.replace(/\s+US$/, "").trim();
  const words = place.split(/\s+/).filter(Boolean).map((w) => {
    if (WORDS[w]) return WORDS[w];
    if (KEEP_UPPER.has(w) || /\d/.test(w)) return w;
    return w
      .toLowerCase()
      .split(/([\-/.'])/)
      .map((part) => (part.length ? part[0].toUpperCase() + part.slice(1) : part))
      .join("");
  });
  const label = words.join(" ");
  return state ? `${label}, ${state}` : label;
}

function milesLabel(mi: number): string {
  const n = Math.round(mi);
  return n < 1 ? "under 1 mi away" : `${n} mi away`;
}

/** The card text, or null outside the season or when no station is within 60 miles. */
export function whiteChristmasCard(dateKey: string, station: WhiteChristmasStation | null): WhiteChristmasCard | null {
  if (!station || !inWhiteChristmasSeason(dateKey)) return null;
  if (!(station.distance_mi <= WHITE_CHRISTMAS_MAX_MI)) return null;
  const pctLabel = `${Math.round(Math.min(100, Math.max(0, station.pct)))}%`;
  const headline = `${pctLabel} of years had snow on the ground on Dec 25`;
  const detail = `at ${stationLabel(station.name)}, ${milesLabel(station.distance_mi)}`;
  return { pctLabel, headline, detail, sentence: `${headline} ${detail}` };
}

/** Station dot colour by percent, as a MapLibre step expression (web and app share it). */
export function whiteChristmasColorExpression(): unknown[] {
  return ["step", ["get", "pct"], "#6b7280", 10, "#7aa7d6", 25, "#5b8def", 40, "#3f6fe0", 50, "#9b7bff", 75, "#d7c9ff", 90, "#ffffff"];
}
