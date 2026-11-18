import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { LAYER_PALETTE_KEY } from "./serverPalettes";
import { WINTER_LAYERS, formatWinterReading, isWinterLayer, winterAsOf, winterLayer, winterPickerRows, winterRowChips, winterSheetHint } from "./winterLayers";

const classic = JSON.parse(
  readFileSync(new URL("../../../rust/crates/radar-tiler/assets/classic.json", import.meta.url), "utf8"),
) as Record<string, unknown>;

describe("winter layers", () => {
  it("lists snow and ice totals, snow depth, 24 h snowfall and snow chance, each with a hint and a server palette", () => {
    expect(WINTER_LAYERS.map((l) => l.id)).toEqual([
      "snowfall-forecast",
      "ice-forecast",
      "snow-depth",
      "snowfall-24h",
      "snow-chance-1",
      "snow-chance-4",
      "snow-chance-8",
    ]);
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
    expect(isWinterLayer("snow-depth")).toBe(true);
    expect(isWinterLayer("hail-size")).toBe(false);
    expect(winterLayer("snowfall-forecast")?.unit).toBe("in");
  });

  it("formats readouts, ice to 0.01 in, depth to whole inches", () => {
    expect(formatWinterReading("snowfall-forecast", 6.24)).toBe("6.2 in");
    expect(formatWinterReading("snowfall-forecast", 0.04)).toBe("No snow");
    expect(formatWinterReading("ice-forecast", 0.256)).toBe("0.26 in");
    expect(formatWinterReading("ice-forecast", 0.004)).toBe("No ice");
    expect(formatWinterReading("snow-depth", 14.49)).toBe("14 in");
    expect(formatWinterReading("snow-depth", 0.6)).toBe("No snow on the ground");
    expect(formatWinterReading("snowfall-24h", 3.06)).toBe("3.1 in");
    expect(formatWinterReading("radar", 5)).toBeNull();
  });

  it("dates observed frames and leaves forecasts alone", () => {
    expect(winterAsOf("snow-depth", "2026-11-17T06:00:00+00:00", "America/Detroit")).toBe("As of Nov 17");
    expect(winterAsOf("snowfall-24h", "2026-11-17T00:00:00+00:00", "America/Detroit")).toBe("As of Nov 16");
    expect(winterAsOf("snowfall-forecast", "2026-11-17T06:00:00+00:00")).toBeNull();
    expect(winterAsOf("snow-depth", undefined)).toBeNull();
    expect(winterAsOf("snow-depth", "nope")).toBeNull();
  });

  it("builds the sheet hint from the selected layer", () => {
    expect(winterSheetHint("snow-depth")).toBe("Snow on the ground, updated once a day · NOAA NOHRSC SNODAS");
    expect(winterSheetHint("radar")).toContain("· NOAA");
  });

  it("folds the three snow chance layers into one row with a 1 / 4 / 8 in chip", () => {
    const all = WINTER_LAYERS;
    expect(winterPickerRows(all, "radar").map((l) => l.id)).toEqual([
      "snowfall-forecast",
      "ice-forecast",
      "snow-depth",
      "snowfall-24h",
      "snow-chance-1",
    ]);
    expect(winterPickerRows(all, "snow-chance-8").at(-1)?.id).toBe("snow-chance-8");
    expect(winterRowChips(all, "snow-chance-4").map((l) => l.chip)).toEqual(["1 in", "4 in", "8 in"]);
    expect(winterRowChips(all, "snow-depth")).toEqual([]);
    // Only the thresholds the server has show; the row stands for the first one.
    const some = all.filter((l) => l.id !== "snow-chance-1");
    expect(winterPickerRows(some, "radar").at(-1)?.id).toBe("snow-chance-4");
    expect(winterPickerRows(all.slice(0, 4), "radar")).toHaveLength(4);
  });

  it("reads snow chance in whole percent", () => {
    expect(formatWinterReading("snow-chance-1", 42.4)).toBe("42%");
    expect(formatWinterReading("snow-chance-8", 100)).toBe("100%");
    expect(formatWinterReading("snow-chance-4", 3)).toBe("Under 5%");
    expect(winterLayer("snow-chance-4")?.credit).toBe("NOAA WPC");
    expect(winterSheetHint("snow-chance-8")).toBe(
      "Chance of at least 8 in of snow in the 24 hours ending at each frame's time · NOAA WPC",
    );
    expect(winterAsOf("snow-chance-1", "2026-11-19T00:00:00+00:00")).toBeNull();
  });
});
