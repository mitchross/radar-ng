import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { layerLegend, type ServerPalettes } from "./layerLegends";

const table = (id: string) =>
  JSON.parse(readFileSync(new URL(`../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url), "utf8"));
const palettes: ServerPalettes = { classic: table("classic"), vivid: table("vivid"), muted: table("muted") };

describe("layerLegend", () => {
  it("draws every renderer band in value order for the HRRR layers", () => {
    for (const layer of ["temperature", "feels-like", "dewpoint", "wind", "precip-accum", "cloud"]) {
      const legend = layerLegend(palettes, "classic", layer);
      expect(legend, layer).not.toBeNull();
      expect(legend!.gradient.startsWith("linear-gradient(90deg,")).toBe(true);
    }
    const temp = layerLegend(palettes, "classic", "temperature")!;
    expect(temp.title).toBe("Temperature (°F)");
    expect(temp.labels[0]).toBe("-40");
    expect(temp.gradient).toContain("rgb(128,0,255) 0.0%");
  });

  it("colors the observed 24-hour total with the rain-total table", () => {
    const observed = layerLegend(palettes, "vivid", "precip-24h")!;
    expect(observed.title).toBe("Past 24 hr precip (in)");
    expect(observed.gradient).toBe(layerLegend(palettes, "vivid", "precip-accum")!.gradient);
    expect(observed.labels).toEqual(["0.01", "8+"]);
  });

  it("returns null when the server tables are missing or the layer has none", () => {
    expect(layerLegend(null, "classic", "temperature")).toBeNull();
    expect(layerLegend(palettes, "classic", "air-quality")).toBeNull();
  });
});
