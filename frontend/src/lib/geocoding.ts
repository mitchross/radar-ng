import { PRECISION, roundCoords } from "./coordinates";
import { fetchWithTimeout } from "./api";
import type { SelectedPlace } from "../types/location";

/**
 * Place search and place labels come from the self-hosted server
 * (`/api/geocode`, `/api/reverse-geocode`, backed by Photon). The app never
 * asks a third-party or platform geocoder, so a position only ever reaches
 * the user's own stack.
 *
 * The reverse result is cached against the rounded position, because several
 * mounted screens ask for the same fix and GPS jitter must not re-resolve it.
 * A cached hit returns the same object, so the store update is a no-op re-render.
 */
const CACHE_TTL_MS = 5 * 60_000;

let cachedKey: string | null = null;
let cachedPlace: SelectedPlace | null = null;
let cachedAt = 0;
let pending: { key: string; promise: Promise<SelectedPlace | null> } | null = null;

/** Test seam: the throttle above is module state and outlives a single test. */
export function resetReverseGeocodeCache(): void {
  cachedKey = null;
  cachedPlace = null;
  cachedAt = 0;
  pending = null;
}

function parsePlace(value: unknown): SelectedPlace | null {
  if (typeof value !== "object" || value === null) return null;
  const p = value as Record<string, unknown>;
  if (
    typeof p.id !== "number" ||
    typeof p.name !== "string" ||
    typeof p.latitude !== "number" ||
    typeof p.longitude !== "number"
  ) {
    return null;
  }
  const optional = (v: unknown) => (typeof v === "string" && v.length > 0 ? v : undefined);
  return {
    id: p.id,
    name: p.name,
    latitude: p.latitude,
    longitude: p.longitude,
    admin1: optional(p.admin1),
    country: optional(p.country),
    countryCode: optional(p.countryCode),
  };
}

async function resolvePlace(serverUrl: string, lat: number, lon: number): Promise<SelectedPlace | null> {
  const res = await fetchWithTimeout(`${serverUrl}/api/reverse-geocode?lat=${lat}&lon=${lon}`);
  if (!res.ok) throw new Error(`Reverse geocode error: ${res.status}`);
  const body = (await res.json()) as { place?: unknown };
  return parsePlace(body.place);
}

export async function reverseGeocode(
  serverUrl: string,
  latitude: number,
  longitude: number,
): Promise<SelectedPlace | null> {
  const { lat, lon } = roundCoords(latitude, longitude, PRECISION.POINT);
  const key = `${serverUrl}|${lat},${lon}`;

  if (key === cachedKey && Date.now() - cachedAt < CACHE_TTL_MS) return cachedPlace;
  // Concurrent callers (one per mounted screen) share a single lookup.
  if (pending?.key === key) return pending.promise;

  const promise = resolvePlace(serverUrl, lat, lon)
    .then((place) => {
      cachedKey = key;
      cachedPlace = place;
      cachedAt = Date.now();
      return place;
    })
    // A missing label is a cosmetic loss, not a screen failure; don't cache it,
    // so a transient server error can still recover on the next fix.
    .catch(() => null);

  pending = { key, promise };
  promise.finally(() => {
    if (pending?.key === key) pending = null;
  });
  return promise;
}

export async function searchCities(
  serverUrl: string,
  query: string,
  signal?: AbortSignal,
): Promise<SelectedPlace[]> {
  const name = query.trim();
  if (name.length < 2) return [];

  const params = new URLSearchParams({ q: name, limit: "8" });
  const response = await fetchWithTimeout(`${serverUrl}/api/geocode?${params.toString()}`, {}, signal);
  if (!response.ok) {
    throw new Error(response.status === 503 ? "City search is not set up on this server" : "City search failed");
  }
  const body = (await response.json()) as { results?: unknown[] };
  return (body.results ?? []).map(parsePlace).filter((p): p is SelectedPlace => p !== null);
}
