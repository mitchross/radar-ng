import { readFileSync } from "fs";
import { join } from "path";
import { LAYER_OPTIONS, SEVERE_OPTIONS, WINTER_OPTIONS, layerOption } from "../../src/components/map/radarLayers";
import { serverLegend, type ServerPalettes } from "../../src/lib/serverPalettes";

const table = (id: string) =>
  JSON.parse(readFileSync(join(__dirname, "../../../rust/crates/radar-tiler/assets", `${id}.json`), "utf8"));
const palettes: ServerPalettes = { classic: table("classic"), muted: table("muted"), vivid: table("vivid") };

describe("Winter layers", () => {
  it("form their own group, separate from the main and severe lists", () => {
    expect(WINTER_OPTIONS.map((o) => o.id)).toEqual(["snowfall-forecast", "ice-forecast", "snow-depth", "snowfall-24h"]);
    for (const o of WINTER_OPTIONS) {
      expect(LAYER_OPTIONS.some((m) => m.id === o.id)).toBe(false);
      expect(SEVERE_OPTIONS.some((m) => m.id === o.id)).toBe(false);
      expect(o.hint.length).toBeGreaterThan(10);
      expect(o.icon).toBeTruthy();
      expect(layerOption(o.id).id).toBe(o.id);
    }
  });

  it("have a legend in every palette family", () => {
    for (const palette of ["classic", "muted", "vivid"] as const) {
      expect(serverLegend(palettes, palette, "snowfall-forecast")).toMatchObject({ title: "Snow Total (in)" });
      expect(serverLegend(palettes, palette, "ice-forecast")?.stops.at(-1)?.label).toBe("0.01");
      expect(serverLegend(palettes, palette, "snow-depth")).toMatchObject({ title: "Snow Depth (in)" });
      expect(serverLegend(palettes, palette, "snow-depth")?.stops.at(-1)?.label).toBe("1");
      expect(serverLegend(palettes, palette, "snowfall-24h")).toMatchObject({ title: "Past 24 Hr Snow (in)" });
    }
  });
});
