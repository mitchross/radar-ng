import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { LAYER_PALETTE_KEY, layerPaletteKey } from "./serverPalettes";

const rust = (path: string) => readFileSync(new URL(`../../../rust/crates/${path}`, import.meta.url), "utf8");

describe("layerPaletteKey", () => {
  it("prefers the key the server sends and falls back to the local map for older servers", () => {
    expect(layerPaletteKey("temperature", "air_temp")).toBe("air_temp");
    expect(layerPaletteKey("temperature")).toBe("temperature");
    expect(layerPaletteKey("temperature", "")).toBe("temperature");
    expect(layerPaletteKey("unknown", null)).toBeUndefined();
  });
});

describe("LAYER_PALETTE_KEY", () => {
  it("is pinned, so changing it is deliberate", () => {
    expect(LAYER_PALETTE_KEY).toEqual({
      temperature: "temperature",
      "feels-like": "temperature",
      dewpoint: "dewpoint",
      wind: "wind_speed",
      "precip-accum": "precip_accum",
      "precip-24h": "precip_accum",
      cloud: "cloud_cover",
      uv: "uv_index",
    });
  });

  it("names a color key every palette serves", () => {
    for (const id of ["classic", "muted", "vivid"]) {
      const palette = JSON.parse(rust(`radar-tiler/assets/${id}.json`)) as Record<string, unknown>;
      for (const key of Object.values(LAYER_PALETTE_KEY)) expect(palette, `${id}.${key}`).toHaveProperty(key);
    }
  });

  it("agrees with the server's HRRR layer_color_key", () => {
    const src = rust("radar-ingest/src/hrrr/state.rs");
    const fn = src.slice(src.indexOf("pub fn layer_color_key"));
    const server: Record<string, string> = {};
    for (const [, layers, key] of fn.slice(0, fn.indexOf("\n}\n")).matchAll(/((?:"[\w-]+"\s*\|?\s*)+)=>\s*"(\w+)"/g)) {
      for (const [, layer] of layers.matchAll(/"([\w-]+)"/g)) server[layer] = key;
    }
    // uv and precip-24h are MRMS/forecast layers, not HRRR ones.
    for (const [layer, key] of Object.entries(LAYER_PALETTE_KEY)) {
      if (layer in server || !["uv", "precip-24h"].includes(layer)) expect(server[layer], layer).toBe(key);
    }
  });
});
