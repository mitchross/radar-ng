/**
 * The camera and screen size at the moment the visible packed-tile range last changed. The in-map
 * engine re-runs its JS tile cover from it, not on every camera frame.
 */
import { useState } from "react";
import { useAnimatedReaction, type SharedValue } from "react-native-reanimated";
import { scheduleOnRN } from "react-native-worklets";
import type { PackedCamera, PackedFrameLocation } from "../../lib/packed";
import type { SharedCamera } from "./WindParticlesOverlay";

export interface CoverView {
  key: string;
  camera: PackedCamera;
  screen: { width: number; height: number };
}

/** Zoom bounds over every packed layer; a cover change at any of them re-runs the JS cover. */
export function packedZoomBounds(locations: readonly (PackedFrameLocation | null)[]) {
  let lo = Infinity;
  let hi = -Infinity;
  for (const loc of locations) {
    if (!loc) continue;
    lo = Math.min(lo, loc.meta.min_zoom);
    hi = Math.max(hi, loc.meta.max_zoom);
  }
  return { lo, hi };
}

/**
 * The worklet body is inlined (helpers across the worklet boundary have dropped calls). The tile
 * zoom rounds, like MapLibre's raster sources (mapLibreRasterTileZoom).
 */
export function usePackedCoverView(
  camera: SharedCamera,
  size: SharedValue<{ width: number; height: number }>,
  zoomBounds: { lo: number; hi: number },
): CoverView | null {
  const [view, setView] = useState<CoverView | null>(null);
  useAnimatedReaction(
    () => {
      "worklet";
      const s = size.get();
      const zoom = camera.zoom.get();
      const base = Math.round(zoom);
      const z = Math.max(zoomBounds.lo, Math.min(zoomBounds.hi, base));
      const world = 512 * Math.pow(2, zoom);
      const latRad = Math.max(-1.4844, Math.min(1.4844, (camera.lat.get() * Math.PI) / 180));
      const cx = ((camera.lon.get() + 180) / 360) * world;
      const cy = (0.5 - Math.log(Math.tan(Math.PI / 4 + latRad / 2)) / (2 * Math.PI)) * world;
      const rad = (camera.bearing.get() * Math.PI) / 180;
      const hw = (s.width / 2) * Math.abs(Math.cos(rad)) + (s.height / 2) * Math.abs(Math.sin(rad));
      const hh = (s.width / 2) * Math.abs(Math.sin(rad)) + (s.height / 2) * Math.abs(Math.cos(rad));
      const tw = 512 * Math.pow(2, zoom - z);
      const key = `${base}|${z}|${Math.floor((cx - hw) / tw)}|${Math.floor((cx + hw) / tw)}|${Math.floor(
        (cy - hh) / tw,
      )}|${Math.floor((cy + hh) / tw)}|${s.width}x${s.height}`;
      return {
        key,
        camera: { lon: camera.lon.get(), lat: camera.lat.get(), zoom, bearing: camera.bearing.get() },
        screen: { width: s.width, height: s.height },
      };
    },
    (next, prev) => {
      if (next.key !== prev?.key) scheduleOnRN(setView, next);
    },
    [zoomBounds],
  );
  return view;
}
