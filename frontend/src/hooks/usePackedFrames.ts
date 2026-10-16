import { useMemo } from "react";
import { useManifestQuery } from "./useManifest";
import { useServerPalettes } from "./useServerPalettes";
import { useWeatherStore } from "../stores/useWeatherStore";
import {
  buildLut,
  locateFrame,
  parsePackedMeta,
  slotAddress,
  type PackedColorTable,
  type PackedLocation,
  type PackedMeta,
} from "../lib/packed";

export interface PackedFrameLocation extends PackedLocation {
  layer: string;
  meta: PackedMeta;
  /** Key into PackedFrames.luts. */
  lutKey: string;
  channel: number;
  x0: number;
  y0: number;
}

export interface PackedFrames {
  /** Per frame index: where the frame lives in packed tiles, or null (raster path). */
  locations: (PackedFrameLocation | null)[];
  isPacked: boolean[];
  /** 256 x 1 RGBA tables (unpremultiplied) by lutKey; a palette change swaps only these. */
  luts: Record<string, Uint8Array>;
}

const lutKeyOf = (meta: PackedMeta) =>
  `${meta.color_key}|${meta.scale}|${meta.offset}|${meta.types?.stride ?? 0}|${meta.types?.classes.join(",") ?? ""}`;

/**
 * Which radar frames the GPU overlay draws. Null when the setting is off, the layer is not
 * radar, or no frame has a packed slot with a palette table to color it.
 */
export function usePackedFrames(): PackedFrames | null {
  const fastRadar = useWeatherStore((s) => s.fastRadar);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const frames = useWeatherStore((s) => s.frames);
  const enabled = fastRadar && (activeLayer === "radar" || activeLayer === "radar-hrrr");
  const manifest = useManifestQuery().data;
  const palettes = useServerPalettes().data;

  return useMemo(() => {
    if (!enabled || !manifest || !palettes || frames.length === 0) return null;
    const tables = palettes[activePalette] ?? palettes.classic;
    const metas = new Map<string, PackedMeta | null>();
    const luts: Record<string, Uint8Array> = {};
    const metaFor = (layer: string): PackedMeta | null => {
      if (!metas.has(layer)) {
        const meta = parsePackedMeta(manifest.layers[layer]?.packed);
        const table = meta ? (tables?.[meta.color_key] as PackedColorTable | undefined) : undefined;
        // No table for the key: leave the layer on rasters rather than draw it blank.
        const usable = meta && table?.ranges?.length ? meta : null;
        if (usable && table) luts[lutKeyOf(usable)] ??= buildLut(table, usable.scale, usable.offset, usable.types);
        metas.set(layer, usable);
      }
      return metas.get(layer) ?? null;
    };
    const locations = frames.map((frame): PackedFrameLocation | null => {
      const layer = frame.source ?? activeLayer;
      const meta = metaFor(layer);
      const loc = locateFrame(meta, frame.timestamp);
      if (!meta || !loc) return null;
      const { channel, x0, y0 } = slotAddress(loc.slot, meta.cols, meta.tile_size);
      return { ...loc, layer, meta, lutKey: lutKeyOf(meta), channel, x0, y0 };
    });
    if (!locations.some(Boolean)) return null;
    return { locations, isPacked: locations.map(Boolean), luts };
  }, [enabled, manifest, palettes, frames, activeLayer, activePalette]);
}
