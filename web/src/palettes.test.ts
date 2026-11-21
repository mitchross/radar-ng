import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { css, PALETTES, radarLegendGradient, typeRamps, type PaletteId } from "./palettes";

describe("palettes", () => {
  it("mirror the tile palettes the backend renders", () => {
    for (const id of Object.keys(PALETTES) as PaletteId[]) {
      const url = new URL(`../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url);
      const table = JSON.parse(readFileSync(url, "utf8")).reflectivity;
      const bands = (ranges: { min: number; rgba: number[] }[]) => ranges.map((r) => ({ min: r.min, rgba: r.rgba }));
      expect(PALETTES[id].bands).toEqual(bands(table.ranges));
      expect(PALETTES[id].snow).toEqual(bands(table.types.snow.ranges));
      expect(PALETTES[id].mix).toEqual(bands(table.types.mix.ranges));
      expect(PALETTES[id].ice).toEqual(bands(table.types.ice.ranges));
    }
  });

  it("builds rain, snow, mix and ice legend ramps", () => {
    const ramps = typeRamps("classic");
    expect(ramps.map((r) => r.label)).toEqual(["Rain", "Snow", "Mix", "Ice"]);
    expect(ramps[3].gradient).toContain(css(PALETTES.classic.ice[0].rgba));
    expect(ramps[1].gradient).toContain(css(PALETTES.classic.snow[0].rgba));
    expect(ramps[2].gradient).toContain(css(PALETTES.classic.mix[6].rgba));
  });

  it("draws the radar legend as flat bands when stepped, the old legend exactly", () => {
    const bands = PALETTES.classic.bands;
    const pos = (dbz: number) => `${((dbz - 5) / (75 - 5)) * 100}%`;
    const old = `linear-gradient(90deg,${bands.map((b, i) => `${css(b.rgba)} ${pos(b.min)} ${pos(bands[i + 1]?.min ?? 75)}`).join(",")})`;
    expect(radarLegendGradient(bands, false)).toBe(old);
  });

  it("draws the radar legend as a gradient through each band start when smooth", () => {
    const g = radarLegendGradient(PALETTES.vivid.bands, true);
    expect(g).toContain(`${css(PALETTES.vivid.bands[0].rgba)} 0%,`);
    expect(g).toContain(`${css(PALETTES.vivid.bands[6].rgba)} ${((35 - 5) / 70) * 100}%,`);
    expect(g.endsWith(`${css(PALETTES.vivid.bands[12].rgba)} 100%)`)).toBe(true);
    // One position per stop: no hard edges.
    expect(g).not.toMatch(/%\s+[\d.]+%/);
    expect(radarLegendGradient(PALETTES.vivid.bands, false)).toMatch(/%\s+[\d.]+%/);
  });

  it("steps the precipitation type ramps only in stepped mode", () => {
    expect(typeRamps("muted", true)).toEqual(typeRamps("muted"));
    const stepped = typeRamps("muted", false)[1].gradient;
    expect(stepped).toContain(`${css(PALETTES.muted.snow[0].rgba)} 0.0% 14.3%`);
    expect(stepped).toContain(`${css(PALETTES.muted.snow[6].rgba)} 85.7% 100.0%`);
  });
});
