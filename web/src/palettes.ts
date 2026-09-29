/**
 * Reflectivity bands per tile palette, mirrored from backend/shared/palettes/*.json
 * (palettes.test.ts fails when they drift). The legend draws these exact bands.
 */
export type PaletteId = "classic" | "muted" | "vivid";

export interface Band {
  min: number;
  rgba: [number, number, number, number];
}

export const PALETTES: Record<PaletteId, { name: string; bands: Band[] }> = {
  classic: {
    name: "Classic",
    bands: [
      { min: 5, rgba: [110, 185, 225, 115] },
      { min: 10, rgba: [90, 200, 195, 155] },
      { min: 15, rgba: [110, 220, 120, 185] },
      { min: 20, rgba: [50, 195, 60, 210] },
      { min: 25, rgba: [0, 180, 0, 210] },
      { min: 30, rgba: [0, 140, 0, 220] },
      { min: 35, rgba: [255, 240, 0, 225] },
      { min: 40, rgba: [255, 170, 0, 230] },
      { min: 45, rgba: [255, 100, 0, 235] },
      { min: 50, rgba: [240, 0, 0, 240] },
      { min: 55, rgba: [180, 0, 0, 245] },
      { min: 60, rgba: [255, 0, 255, 245] },
      { min: 65, rgba: [180, 0, 220, 250] },
    ],
  },
  muted: {
    name: "Muted",
    bands: [
      { min: 5, rgba: [120, 95, 150, 60] },
      { min: 10, rgba: [95, 50, 125, 100] },
      { min: 15, rgba: [68, 1, 84, 140] },
      { min: 20, rgba: [72, 35, 116, 170] },
      { min: 25, rgba: [59, 82, 139, 195] },
      { min: 30, rgba: [49, 104, 142, 210] },
      { min: 35, rgba: [33, 144, 141, 220] },
      { min: 40, rgba: [53, 183, 121, 225] },
      { min: 45, rgba: [94, 201, 97, 230] },
      { min: 50, rgba: [194, 223, 34, 235] },
      { min: 55, rgba: [253, 231, 37, 240] },
      { min: 60, rgba: [240, 180, 40, 245] },
      { min: 65, rgba: [210, 110, 30, 250] },
    ],
  },
  vivid: {
    name: "Vivid",
    bands: [
      { min: 5, rgba: [215, 238, 250, 75] },
      { min: 10, rgba: [198, 234, 255, 115] },
      { min: 15, rgba: [180, 230, 255, 150] },
      { min: 20, rgba: [80, 200, 255, 190] },
      { min: 25, rgba: [0, 170, 255, 210] },
      { min: 30, rgba: [0, 230, 220, 220] },
      { min: 35, rgba: [120, 255, 180, 225] },
      { min: 40, rgba: [255, 220, 80, 230] },
      { min: 45, rgba: [255, 160, 90, 235] },
      { min: 50, rgba: [255, 100, 170, 240] },
      { min: 55, rgba: [240, 60, 200, 245] },
      { min: 60, rgba: [220, 60, 220, 245] },
      { min: 65, rgba: [140, 40, 255, 250] },
    ],
  },
};

export const css = ([r, g, b, a]: Band["rgba"]) => `rgba(${r},${g},${b},${(Math.max(a, 150) / 255).toFixed(2)})`;

/** Legend rows: drizzle → hail, with the dBZ each label starts at. */
export const LEGEND_TICKS: [number, string][] = [
  [5, "Drizzle"],
  [25, "Rain"],
  [40, "Heavy"],
  [60, "Hail"],
];

export function colorForDbz(palette: PaletteId, dbz: number): string {
  const bands = PALETTES[palette].bands;
  let hit = bands[0];
  for (const b of bands) if (dbz >= b.min) hit = b;
  return css(hit.rgba);
}
