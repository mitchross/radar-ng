import type { LayerType, Palette } from "../types/weather";

/** Tile URL template for MapLibre RasterSource. */
export function buildSelfHostedTileUrl(
  serverUrl: string,
  layer: LayerType | "nowcast",
  timestamp: string,
  palette: Palette = "classic",
): string {
  return `${serverUrl}/tiles/${layer}/${palette}/${timestamp}/{z}/{x}/{y}.png`;
}
