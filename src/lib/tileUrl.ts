import type { RadarFrame } from "../types/weather";
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
