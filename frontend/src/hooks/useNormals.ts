/** Normal high and low for today at a point, from our server. Lower 48 only; `null` until built. */
import { useQuery } from "@tanstack/react-query";
import { fetchWithTimeout } from "../lib/api";
import { inLower48, normalsPath, parseNormals, type DayNormals } from "../lib/tempNormal";

export async function fetchNormals(serverUrl: string, lat: number, lon: number, date: string, signal?: AbortSignal): Promise<DayNormals | null> {
  const res = await fetchWithTimeout(`${serverUrl}${normalsPath(lat, lon, date)}`, {}, signal);
  // 404: no normal here; 503: not built yet. Both hide the line.
  if (res.status === 404 || res.status === 503) return null;
  if (!res.ok) throw new Error(`normals ${res.status}`);
  return parseNormals(await res.json());
}

export function useNormals(serverUrl: string, lat: number, lon: number, date: string | undefined) {
  const [rlat, rlon] = [Math.round(lat * 100) / 100, Math.round(lon * 100) / 100];
  const enabled = !!date && inLower48(lat, lon);
  return useQuery({
    queryKey: ["normals", serverUrl, rlat, rlon, date],
    queryFn: ({ signal }) => fetchNormals(serverUrl, rlat, rlon, date as string, signal),
    enabled,
    staleTime: 6 * 60 * 60_000,
    gcTime: 24 * 60 * 60_000,
  });
}
