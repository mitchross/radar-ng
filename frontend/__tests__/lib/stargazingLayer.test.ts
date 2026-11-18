import { LAYER_OPTIONS, layerOption, liveLayerOptions } from "../../src/components/map/radarLayers";
import { formatReading } from "../../src/lib/inspector";
import { STARGAZING_LAYER, skyQualityNote } from "../../src/lib/stargazingLayer";

describe("Stargazing map layer", () => {
  it("is a main layer the sheet can list when the manifest has it", () => {
    const o = layerOption("stargazing");
    expect(o.id).toBe("stargazing");
    expect(o.name).toBe(STARGAZING_LAYER.name);
    expect(LAYER_OPTIONS.some((m) => m.id === "stargazing")).toBe(true);
  });

  it("is hidden until the manifest has frames for it", () => {
    expect(liveLayerOptions(undefined).some((m) => m.id === "stargazing")).toBe(false);
    expect(liveLayerOptions({ stargazing: { frames: [] } }).some((m) => m.id === "stargazing")).toBe(false);
    expect(liveLayerOptions({ stargazing: { frames: [{}] } }).some((m) => m.id === "stargazing")).toBe(true);
  });

  it("reads as a word and score", () => {
    expect(formatReading("stargazing", { ok: true, value: 81, unit: "score", source: "hrrr" } as never)).toBe("Excellent · 81");
    expect(formatReading("stargazing", { ok: false, value: null, unit: "", source: "unavailable" } as never)).toBe("\u2014");
  });

  it("credits ECCC in the legend and the inspector note", () => {
    expect(STARGAZING_LAYER.seeingCredit).toContain("Data Source: Environment and Climate Change Canada");
    expect(skyQualityNote({ ok: true, value: 70, seeing: 3, transparency: 4 })).toBe(
      "Seeing Average · Transparency Good · Data Source: Environment and Climate Change Canada",
    );
  });
});
