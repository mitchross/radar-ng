/** Sky tonight from our server (ISS passes) plus the ISS notification opt-in. */
import { useCallback, useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchWithTimeout } from "../lib/api";
import { PRECISION, roundCoord } from "../lib/coordinates";
import { issAlertsEnabled, setIssAlertsEnabled, syncIssAlerts } from "../lib/issAlertScheduler";
import { ensureNotificationPermission } from "../lib/rainAlertScheduler";
import { parseSkyTonight, planIssAlerts, skyTonightPath, type HourlyClouds, type IssReport, type SkyClock } from "../lib/skyTonight";

export async function fetchSkyTonight(serverUrl: string, lat: number, lon: number, signal?: AbortSignal): Promise<IssReport | null> {
  const res = await fetchWithTimeout(`${serverUrl}${skyTonightPath(lat, lon)}`, {}, signal);
  if (!res.ok) throw new Error(`sky tonight ${res.status}`);
  return parseSkyTonight(await res.json());
}

export function useSkyTonight(serverUrl: string, lat: number, lon: number) {
  const [rlat, rlon] = [roundCoord(lat, PRECISION.WEATHER), roundCoord(lon, PRECISION.WEATHER)];
  return useQuery({
    queryKey: ["sky-tonight", serverUrl, rlat, rlon],
    queryFn: ({ signal }) => fetchSkyTonight(serverUrl, rlat, rlon, signal),
    staleTime: 30 * 60_000,
  });
}

/** Turning ISS alerts on asks for notification permission and persists only when granted. */
export function useIssAlertOptIn() {
  const [enabled, setEnabled] = useState(issAlertsEnabled);
  const [blocked, setBlocked] = useState(false);
  const setIssAlerts = useCallback(async (on: boolean) => {
    if (!on) {
      setIssAlertsEnabled(false);
      setEnabled(false);
      setBlocked(false);
      await syncIssAlerts([]);
      return;
    }
    const granted = await ensureNotificationPermission(true);
    setBlocked(!granted);
    setIssAlertsEnabled(granted);
    setEnabled(granted);
  }, []);
  return { enabled, blocked, setIssAlerts };
}

/** While the card is mounted, keep the scheduled ISS notifications in step with the passes. */
export function useIssAlertSync(report: IssReport | null | undefined, hourly: HourlyClouds | null, clock: SkyClock) {
  const { timeZone, utcOffsetSeconds } = clock;
  useEffect(() => {
    if (report === undefined) return;
    const plans = issAlertsEnabled() ? planIssAlerts(report, hourly, Date.now(), { timeZone, utcOffsetSeconds }) : [];
    void syncIssAlerts(plans);
  }, [report, hourly, timeZone, utcOffsetSeconds]);
}
