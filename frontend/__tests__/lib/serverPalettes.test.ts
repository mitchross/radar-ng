import { readFileSync } from "fs";
import { join } from "path";
import { serverLegend, type ServerPalettes } from "../../src/lib/serverPalettes";

const table = (id: string) =>
  JSON.parse(readFileSync(join(__dirname, "../../../rust/crates/radar-tiler/assets", `${id}.json`), "utf8"));
const palettes: ServerPalettes = { classic: table("classic"), muted: table("muted"), vivid: table("vivid") };

describe("serverLegend", () => {
  it("builds top-to-bottom stops from the renderer tables", () => {
    const temp = serverLegend(palettes, "classic", "temperature")!;
    expect(temp.title).toBe("Temperature (°F)");
    expect(temp.stops[temp.stops.length - 1]).toEqual({ label: "-40", color: "#8000ff" });
    expect(temp.stops[0].label).toBe("110+");
    expect(temp.stops.length).toBeLessThanOrEqual(7);
    for (const layer of ["wind", "precip-accum", "cloud"] as const) {
      expect(serverLegend(palettes, "vivid", layer)).not.toBeNull();
    }
  });

  it("labels the UV legend with whole WHO index values", () => {
    for (const palette of ["classic", "muted", "vivid"] as const) {
      const uv = serverLegend(palettes, palette, "uv")!;
      expect(uv.title).toBe("UV Index (WHO)");
      expect(uv.stops.map((s) => s.label)).toEqual(["11+", "8", "6", "3", "1"]);
    }
    expect(serverLegend(palettes, "classic", "uv")!.stops[4].color).toBe("#289500");
  });

  it("falls back to null without tables or for layers it does not map", () => {
    expect(serverLegend(undefined, "classic", "temperature")).toBeNull();
    expect(serverLegend(palettes, "classic", "radar")).toBeNull();
  });
});
