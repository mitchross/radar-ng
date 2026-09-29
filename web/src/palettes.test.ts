import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { PALETTES, type PaletteId } from "./palettes";

describe("palettes", () => {
  it("mirror the tile palettes the backend renders", () => {
    for (const id of Object.keys(PALETTES) as PaletteId[]) {
      const url = new URL(`../../backend/shared/palettes/${id}.json`, import.meta.url);
      const ranges = JSON.parse(readFileSync(url, "utf8")).reflectivity.ranges as { min: number; rgba: number[] }[];
      expect(PALETTES[id].bands).toEqual(ranges.map((r) => ({ min: r.min, rgba: r.rgba })));
    }
  });
});
