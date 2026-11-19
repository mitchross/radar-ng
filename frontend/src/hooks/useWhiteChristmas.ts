/** White Christmas odds from our server: the nearest station for the card, every station for the map dots. */
import { useQuery } from "@tanstack/react-query";
import { fetchWithTimeout } from "../lib/api";
import { PRECISION, roundCoord } from "../lib/coordinates";
import {
  parseNearestStation,
  WHITE_CHRISTMAS_STATIONS_PATH,
  whiteChristmasPath,
  type WhiteChristmasStation,
} from "../lib/whiteChristmas";

/** The table is fixed climate normals, so cache it for the session. */
const FOREVER = Number.POSITIVE_INFINITY;

export async function fetchWhiteChristmasNearest(
  serverUrl: string,
  lat: number,
  lon: number,
  signal?: AbortSignal,
): Promise<WhiteChristmasStation | null> {
  const res = await fetchWithTimeout(`${serverUrl}${whiteChristmasPath(lat, lon)}`, {}, signal);
  if (!res.ok) throw new Error(`white christmas ${res.status}`);
  return parseNearestStation(await res.json());
}

export async function fetchWhiteChristmasStations(serverUrl: string, signal?: AbortSignal): Promise<GeoJSON.FeatureCollection> {
  const res = await fetchWithTimeout(`${serverUrl}${WHITE_CHRISTMAS_STATIONS_PATH}`, {}, signal, 30_000);
  if (!res.ok) throw new Error(`white christmas stations ${res.status}`);
  return res.json();
}

export function useWhiteChristmasNearest(serverUrl: string, lat: number, lon: number, enabled: boolean) {
  const [rlat, rlon] = [roundCoord(lat, PRECISION.WEATHER), roundCoord(lon, PRECISION.WEATHER)];
  return useQuery({
    queryKey: ["white-christmas", serverUrl, rlat, rlon],
    queryFn: ({ signal }) => fetchWhiteChristmasNearest(serverUrl, rlat, rlon, signal),
    enabled,
    staleTime: FOREVER,
    gcTime: 24 * 60 * 60_000,
  });
}

export function useWhiteChristmasStations(serverUrl: string, enabled: boolean) {
  return useQuery({
    queryKey: ["white-christmas-stations", serverUrl],
    queryFn: ({ signal }) => fetchWhiteChristmasStations(serverUrl, signal),
    enabled,
    staleTime: FOREVER,
    gcTime: 24 * 60 * 60_000,
  });
}
