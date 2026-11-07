import { readFileSync } from "fs";
import { join } from "path";
import { LAYER_OPTIONS, SEVERE_OPTIONS, layerOption } from "../../src/components/map/radarLayers";
import { serverLegend, type ServerPalettes } from "../../src/lib/serverPalettes";

const table = (id: string) =>
  JSON.parse(readFileSync(join(__dirname, "../../../rust/crates/radar-tiler/assets", `${id}.json`), "utf8"));
const palettes: ServerPalettes = { classic: table("classic"), muted: table("muted"), vivid: table("vivid") };

describe("Severe layers", () => {
  it("form their own group, separate from the main layer list", () => {
    expect(SEVERE_OPTIONS.map((o) => o.id)).toEqual(["hail-size", "rotation-track", "echo-top"]);
    for (const o of SEVERE_OPTIONS) {
      expect(LAYER_OPTIONS.some((m) => m.id === o.id)).toBe(false);
      expect(o.hint.length).toBeGreaterThan(10);
      expect(layerOption(o.id).id).toBe(o.id);
    }
  });

  it("have a legend in every palette family", () => {
    for (const palette of ["classic", "muted", "vivid"] as const) {
      expect(serverLegend(palettes, palette, "hail-size")).toMatchObject({ title: "Hail Size (in)" });
      expect(serverLegend(palettes, palette, "rotation-track")?.stops.at(-1)?.label).toBe("0.003");
      expect(serverLegend(palettes, palette, "echo-top")?.stops[0].label).toBe("65+");
    }
  });
});
