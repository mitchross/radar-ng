/** Snow at your spot from our server, plus whether the Home card is showing (the map sheet opens Winter first then). */
import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { create } from "zustand";
import { fetchWithTimeout } from "../lib/api";
import { parseSnowSpot, snowSpotPath, type SnowSpot } from "../lib/snowSpot";

export async function fetchSnowSpot(serverUrl: string, lat: number, lon: number, signal?: AbortSignal): Promise<SnowSpot | null> {
  const res = await fetchWithTimeout(`${serverUrl}${snowSpotPath(lat, lon)}`, {}, signal);
  if (!res.ok) throw new Error(`snow spot ${res.status}`);
  return parseSnowSpot(await res.json());
}

export function useSnowSpot(serverUrl: string, lat: number, lon: number) {
  // The server caches per 0.1° cell, so the query key uses the same cell.
  const [rlat, rlon] = [Math.round(lat * 10) / 10, Math.round(lon * 10) / 10];
  return useQuery({
    queryKey: ["snow-spot", serverUrl, rlat, rlon],
    queryFn: ({ signal }) => fetchSnowSpot(serverUrl, rlat, rlon, signal),
    staleTime: 15 * 60_000,
    gcTime: 60 * 60_000,
  });
}

export const useSnowCardVisible = create<{ visible: boolean }>(() => ({ visible: false }));

/** Publish the card's visibility while it is mounted. */
export function useSnowCardVisibility(visible: boolean) {
  useEffect(() => {
    useSnowCardVisible.setState({ visible });
    return () => useSnowCardVisible.setState({ visible: false });
  }, [visible]);
}
