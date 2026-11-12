import { useCallback, useEffect, useRef } from "react";
import { useIsFocused } from "expo-router";
import { fetchWithTimeout } from "../lib/api";
import { fallbackCopy, Level2Session, type Level2Palette, type Viewport } from "../lib/level2";
import { isReplaySource } from "../lib/replaySource";
import { NO_LEVEL2, useWeatherStore } from "../stores/useWeatherStore";
import { useAppActive } from "./useAppActive";

export interface Level2Camera {
  lon: number;
  lat: number;
  zoom: number;
  /** West, south, east, north. */
  bounds: [number, number, number, number];
}

const round2 = (v: number) => Math.round(v * 100) / 100;

/**
 * One Level II session for the radar tab. Feeds it the camera, palette and layer, and publishes
 * its snapshot to the store; every request goes to the configured server and nowhere else.
 */
export function useLevel2(): { onCamera: (c: Level2Camera) => void } {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const palette = useWeatherStore((s) => s.activePalette);
  const replaying = useWeatherStore((s) => isReplaySource(s.source));
  const focused = useIsFocused();
  const appActive = useAppActive();
  const session = useRef<Level2Session | null>(null);
  const camera = useRef<Level2Camera | null>(null);
  const published = useRef("");


  const active = activeLayer === "radar" && !replaying && focused && appActive;
  const apply = useCallback(() => {
    const c = camera.current;
    if (!session.current || !c) return;
    const vp: Viewport = { centre: { lat: c.lat, lon: c.lon }, zoom: c.zoom, bounds: c.bounds, palette: palette as Level2Palette, active };
    session.current.update(vp);
  }, [palette, active]);

  useEffect(() => {
    const s = new Level2Session({
      request: async (path, init) => {
        const res = await fetchWithTimeout(`${serverUrl}${path}`, { method: init.method, body: init.body, headers: init.body ? { "content-type": "application/json" } : undefined }, init.signal);
        let json: unknown = null;
        try {
          json = await res.json();
        } catch {
          json = null;
        }
        return { status: res.status, json };
      },
    });
    session.current = s;
    published.current = "";
    s.subscribe(() => {
      const snap = s.snapshot();
      const c = camera.current;
      const centre = c ? { lat: round2(c.lat), lon: round2(c.lon) } : null;
      const key = `${snap.key}|${centre ? `${centre.lat},${centre.lon}` : ""}`;
      if (key === published.current) return;
      published.current = key;
      useWeatherStore.getState().setLevel2({ view: snap.view, opacity: snap.opacity, fallback: snap.unavailable ? fallbackCopy(snap.reason) : null, centre });
    });
    apply();
    return () => {
      s.stop();
      session.current = null;
      useWeatherStore.getState().setLevel2(NO_LEVEL2);
    };
  }, [serverUrl, apply]);

  useEffect(apply, [apply]);

  const onCamera = useCallback(
    (c: Level2Camera) => {
      camera.current = c;
      apply();
    },
    [apply],
  );
  return { onCamera };
}
