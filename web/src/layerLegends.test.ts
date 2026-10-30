import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { layerLegend, uvCategory } from "./layerLegends";
import type { ServerPalettes } from "./shared/serverPalettes";

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

  it("labels the UV legend with whole WHO index values in every palette", () => {
    for (const palette of ["classic", "muted", "vivid"] as const) {
      const uv = layerLegend(palettes, palette, "uv")!;
      expect(uv.title).toBe("UV index (WHO)");
      expect(uv.labels).toEqual(["1", "11+"]);
    }
    expect(layerLegend(palettes, "classic", "uv")!.gradient).toContain("rgb(40,149,0)");
  });

  it("names WHO categories on the rounded index", () => {
    expect([0, 2.4, 2.5, 5.4, 6, 7.4, 8, 10.4, 10.5, 14].map(uvCategory)).toEqual([
      "Low",
      "Low",
      "Moderate",
      "Moderate",
      "High",
      "High",
      "Very high",
      "Very high",
      "Extreme",
      "Extreme",
    ]);
  });

  it("colors the observed 24-hour total with the rain-total table", () => {
    const observed = layerLegend(palettes, "vivid", "precip-24h")!;
    expect(observed.title).toBe("Past 24 hr precip (in)");
    expect(observed.gradient).toBe(layerLegend(palettes, "vivid", "precip-accum")!.gradient);
    expect(observed.labels).toEqual(["0.01", "8+"]);
  });

  it("uses the palette key the server sends, and the local map without one", () => {
    const sent = layerLegend(palettes, "classic", "temperature", "dewpoint")!;
    expect(sent.gradient).toBe(layerLegend(palettes, "classic", "dewpoint")!.gradient);
    expect(sent.gradient).not.toBe(layerLegend(palettes, "classic", "temperature")!.gradient);
    expect(layerLegend(palettes, "classic", "temperature", undefined)).toEqual(layerLegend(palettes, "classic", "temperature"));
    expect(layerLegend(palettes, "classic", "temperature", "no_such_key")).toBeNull();
  });

  it("returns null when the server tables are missing or the layer has none", () => {
    expect(layerLegend(null, "classic", "temperature")).toBeNull();
    expect(layerLegend(palettes, "classic", "air-quality")).toBeNull();
  });
});
