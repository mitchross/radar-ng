import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { css, PALETTES, typeRamps, type PaletteId } from "./palettes";

describe("palettes", () => {
  it("mirror the tile palettes the backend renders", () => {
    for (const id of Object.keys(PALETTES) as PaletteId[]) {
      const url = new URL(`../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url);
      const table = JSON.parse(readFileSync(url, "utf8")).reflectivity;
      const bands = (ranges: { min: number; rgba: number[] }[]) => ranges.map((r) => ({ min: r.min, rgba: r.rgba }));
      expect(PALETTES[id].bands).toEqual(bands(table.ranges));
      expect(PALETTES[id].snow).toEqual(bands(table.types.snow.ranges));
      expect(PALETTES[id].mix).toEqual(bands(table.types.mix.ranges));
    }
  });

  it("builds rain, snow and mix legend ramps", () => {
    const ramps = typeRamps("classic");
    expect(ramps.map((r) => r.label)).toEqual(["Rain", "Snow", "Mix"]);
    expect(ramps[1].gradient).toContain(css(PALETTES.classic.snow[0].rgba));
    expect(ramps[2].gradient).toContain(css(PALETTES.classic.mix[6].rgba));
  });
});
