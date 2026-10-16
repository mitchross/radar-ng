import { useMemo } from "react";
import { useManifestQuery } from "./useManifest";
import { useServerPalettes } from "./useServerPalettes";
import { useWeatherStore } from "../stores/useWeatherStore";
import { buildPackedFrames, type PackedFrames } from "../lib/packed";

export type { PackedFrameLocation, PackedFrames } from "../lib/packed";

/**
 * Which frames of the active layer the GPU overlay draws: radar, HRRR and AQM layers with packed
 * metadata. Null when the setting is off or no frame has a packed slot (raster fallback).
 */
export function usePackedFrames(): PackedFrames | null {
  const enabled = useWeatherStore((s) => s.fastRadar);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const frames = useWeatherStore((s) => s.frames);
  const manifest = useManifestQuery().data;
  const palettes = useServerPalettes().data;

  return useMemo(() => {
    if (!enabled || !manifest || !palettes || frames.length === 0) return null;
    const tables = palettes[activePalette] ?? palettes.classic;
    return buildPackedFrames(frames, manifest.layers, tables, activeLayer);
  }, [enabled, manifest, palettes, frames, activeLayer, activePalette]);
}
