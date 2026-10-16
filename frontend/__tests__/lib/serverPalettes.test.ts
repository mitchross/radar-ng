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

  it("colors the past-24-hour layer with the rain-total table", () => {
    const observed = serverLegend(palettes, "classic", "precip-24h")!;
    expect(observed.title).toBe("Past 24 Hr (in)");
    expect(observed.stops).toEqual(serverLegend(palettes, "classic", "precip-accum")!.stops);
    expect(observed.stops[0].label).toBe("8+");
  });

  it("falls back to null without tables or for layers it does not map", () => {
    expect(serverLegend(undefined, "classic", "temperature")).toBeNull();
    expect(serverLegend(palettes, "classic", "radar")).toBeNull();
  });
});
