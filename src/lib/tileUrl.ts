import type { RadarFrame, LayerType, Palette } from "../types/weather";
import { RADAR } from "./constants";

export function buildRadarTileUrl(
  host: string,
  frame: RadarFrame,
  options: {
    size?: number;
    color?: number;
    smooth?: boolean;
    snow?: boolean;
  } = {}
): string {
  const {
    size = RADAR.TILE_SIZE,
    color = RADAR.COLOR_SCHEME,
    smooth = RADAR.SMOOTH,
    snow = RADAR.SNOW,
  } = options;
  return `${host}${frame.path}/${size}/{z}/{x}/{y}/${color}/${smooth ? 1 : 0}_${snow ? 1 : 0}.png`;
}

export function buildIEMTileUrl(product: string): string {
  return `https://mesonet.agron.iastate.edu/cache/tile.py/1.0.0/${product}/{z}/{x}/{y}.png`;
}

export function buildSelfHostedTileUrl(
  serverUrl: string,
  layer: LayerType | "nowcast",
  timestamp: string,
  palette: Palette = "classic",
): string {
  // Palette segment is optional on the server side — Caddy/tile-cleanup
  // accept both `/tiles/{layer}/{timestamp}/...` (legacy) and
  // `/tiles/{layer}/{palette}/{timestamp}/...` (multi-palette).
  return `${serverUrl}/tiles/${layer}/${palette}/${timestamp}/{z}/{x}/{y}.png`;
}
