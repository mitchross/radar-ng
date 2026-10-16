import { Layer } from "@maplibre/maplibre-react-native";
import { useMemo } from "react";
import { useManifestQuery } from "../../hooks/useManifest";
import { clampWindow, type PlaybackWindow } from "../../lib/radarCarousel";
import {
  SATELLITE_OPACITY,
  satelliteFrames,
  satelliteIndexAt,
  satelliteSequence,
  satelliteTileUrl,
} from "../../lib/satellite";
import { useWeatherStore } from "../../stores/useWeatherStore";
import type { RadarFrame } from "../../types/weather";
import { RasterFrameCarousel, type CarouselFrameSpec } from "./RasterFrameCarousel";

/**
 * Satellite slots insert under this invisible layer. It mounts first and never
 * unmounts, so radar slots, which append on top as they remount, stay above it.
 */
export const SATELLITE_ANCHOR_ID = "satellite-anchor";

/**
 * GOES satellite imagery under the radar: the frame at or before the timeline
 * frame, with the frames playback reaches next loading in hidden slots.
 * Mount as the first map child.
 */
export function SatelliteOverlay() {
  const visible = useWeatherStore((s) => s.satelliteVisible);
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const playbackWindow = useWeatherStore((s) => s.playbackWindow);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const manifest = useManifestQuery().data;

  const satellite = useMemo(() => satelliteFrames(manifest), [manifest]);
  const satFrames = useMemo<RadarFrame[]>(
    () => satellite.map((f) => ({ time: f.time, timestamp: f.path, path: f.path, maxZoom: f.maxZoom })),
    [satellite],
  );
  // Satellite frames in the order playback visits them, so the hidden slots hold what shows next.
  const satWindow = useMemo<PlaybackWindow | null>(() => {
    if (frames.length === 0) return null;
    const { start, end, sequence } = clampWindow(playbackWindow, frames.length);
    const order = sequence ?? Array.from({ length: end - start + 1 }, (_, k) => start + k);
    const visits = satelliteSequence(satellite, order.map((i) => frames[i].time));
    return visits.length > 0
      ? { start: Math.min(...visits), end: Math.max(...visits), sequence: visits }
      : null;
  }, [frames, playbackWindow, satellite]);

  const anchor = (
    <Layer key={SATELLITE_ANCHOR_ID} id={SATELLITE_ANCHOR_ID} type="background" paint={{ "background-opacity": 0 }} />
  );
  const timelineFrame = frames[currentFrameIndex];
  if (!visible || satFrames.length === 0 || !timelineFrame) return [anchor];

  const index = satelliteIndexAt(satellite, timelineFrame.time);
  // Too old for this timeline time: keep the slots mounted (constant child count) but hidden.
  const shown = index >= 0 ? index : (satWindow?.sequence?.[0] ?? satFrames.length - 1);
  const specFor = (frame: RadarFrame): CarouselFrameSpec => ({
    tileUrl: satelliteTileUrl(serverUrl, frame.path),
    maxZoom: frame.maxZoom ?? 7,
    variant: "goes",
  });

  return [
    anchor,
    <RasterFrameCarousel
      key="satellite-carousel"
      idPrefix="satellite"
      frames={satFrames}
      currentFrameIndex={shown}
      playbackWindow={satWindow}
      opacity={index >= 0 ? SATELLITE_OPACITY : 0}
      minZoom={0}
      specFor={specFor}
      beforeId={SATELLITE_ANCHOR_ID}
    />,
  ];
}
