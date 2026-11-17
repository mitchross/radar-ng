import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { LAYER_PALETTE_KEY } from "./serverPalettes";
import { WINTER_LAYERS, formatWinterReading, isWinterLayer, winterLayer } from "./winterLayers";

const classic = JSON.parse(
  readFileSync(new URL("../../../rust/crates/radar-tiler/assets/classic.json", import.meta.url), "utf8"),
) as Record<string, unknown>;

describe("winter layers", () => {
  it("lists snow and ice totals, each with a hint and a server palette", () => {
    expect(WINTER_LAYERS.map((l) => l.id)).toEqual(["snowfall-forecast", "ice-forecast"]);
    for (const l of WINTER_LAYERS) {
      expect(l.hint.length).toBeGreaterThan(10);
      expect(LAYER_PALETTE_KEY[l.id]).toBe(l.paletteKey);
      expect(classic[l.paletteKey]).toBeTruthy();
    }
  });

  it("agrees with the server's winter layer table", () => {
    const src = readFileSync(new URL("../../../rust/crates/radar-ingest/src/winter/mod.rs", import.meta.url), "utf8");
    const server = Object.fromEntries([...src.matchAll(/id: "([\w-]+)",\s*color_key: "(\w+)"/g)].map(([, id, key]) => [id, key]));
    expect(server).toEqual(Object.fromEntries(WINTER_LAYERS.map((l) => [l.id, l.paletteKey])));
  });

  it("recognises only winter layers", () => {
    expect(isWinterLayer("ice-forecast")).toBe(true);
    expect(isWinterLayer("hail-size")).toBe(false);
    expect(winterLayer("snowfall-forecast")?.unit).toBe("in");
  });

  it("formats readouts, ice to 0.01 in", () => {
    expect(formatWinterReading("snowfall-forecast", 6.24)).toBe("6.2 in");
    expect(formatWinterReading("snowfall-forecast", 0.04)).toBe("No snow");
    expect(formatWinterReading("ice-forecast", 0.256)).toBe("0.26 in");
    expect(formatWinterReading("ice-forecast", 0.004)).toBe("No ice");
    expect(formatWinterReading("radar", 5)).toBeNull();
  });
});
