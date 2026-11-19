import { LAYER_OPTIONS, layerOption, liveLayerOptions } from "../../src/components/map/radarLayers";
import { formatReading } from "../../src/lib/inspector";
import { NICE_DAY_LAYER } from "../../src/lib/niceDayLayer";

describe("Nice Day map layer", () => {
  it("is a main layer the sheet can list when the manifest has it", () => {
    const o = layerOption("nice-day");
    expect(o.id).toBe("nice-day");
    expect(o.name).toBe(NICE_DAY_LAYER.name);
    expect(LAYER_OPTIONS.some((m) => m.id === "nice-day")).toBe(true);
  });

  it("is hidden until the manifest has frames for it", () => {
    expect(liveLayerOptions(undefined).some((m) => m.id === "nice-day")).toBe(false);
    expect(liveLayerOptions({ "nice-day": { frames: [] } }).some((m) => m.id === "nice-day")).toBe(false);
    expect(liveLayerOptions({ "nice-day": { frames: [{}] } }).some((m) => m.id === "nice-day")).toBe(true);
    expect(liveLayerOptions(undefined).some((m) => m.id === "radar")).toBe(true);
  });

  it("reads as the card's word and score", () => {
    expect(formatReading("nice-day", { ok: true, value: 72, unit: "score", source: "hrrr" } as never)).toBe("Good · 72");
    expect(formatReading("nice-day", { ok: true, value: 5, unit: "score", source: "hrrr" } as never)).toBe("Do Not · 5");
    expect(formatReading("nice-day", { ok: false, value: null, unit: "", source: "unavailable" } as never)).toBe("\u2014");
  });
});
