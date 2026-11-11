import { REPLAY_PALETTE, replayTileUrl } from "./replaySource";
import type { LayerType, Palette } from "../types/weather";

/** Replay tile template (/replays/{case}/{rev}/tiles/...) filled for one frame; classic palette only. */
export function buildReplayTileUrl(serverUrl: string, template: string, layer: string, path: string): string {
  return `${serverUrl}${replayTileUrl(template, layer, REPLAY_PALETTE, path)}`;
}

/** Tile URL template for MapLibre RasterSource. */
export function buildSelfHostedTileUrl(
  serverUrl: string,
  layer: LayerType | "nowcast" | "radar-gfs",
  timestamp: string,
  palette: Palette = "classic",
): string {
  return `${serverUrl}/tiles/${layer}/${palette}/${timestamp}/{z}/{x}/{y}.png`;
}
