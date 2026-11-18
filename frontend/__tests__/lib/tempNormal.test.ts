import { LAYER_OPTIONS, layerOption, liveLayerOptions } from "../../src/components/map/radarLayers";
import { formatReading } from "../../src/lib/inspector";
import { TEMP_NORMAL_LAYER, normalsLine, parseNormals } from "../../src/lib/tempNormal";
import { fetchNormals } from "../../src/hooks/useNormals";

const contract = require("../../../web/src/shared/testdata/normals.contract.json");

describe("Temp vs normal", () => {
  it("is a main layer, hidden until the manifest has frames", () => {
    expect(layerOption("temp-anomaly").name).toBe(TEMP_NORMAL_LAYER.name);
    expect(LAYER_OPTIONS.some((m) => m.id === "temp-anomaly")).toBe(true);
    expect(liveLayerOptions(undefined).some((m) => m.id === "temp-anomaly")).toBe(false);
    expect(liveLayerOptions({ "temp-anomaly": { frames: [{}] } }).some((m) => m.id === "temp-anomaly")).toBe(true);
  });

  it("reads a map value in words", () => {
    expect(formatReading("temp-anomaly", { ok: true, value: 12.4 } as never)).toBe("12\u00B0 above normal");
  });

  it("builds the home line from the server contract, in the chosen unit", () => {
    const n = parseNormals(contract.detroit);
    expect(normalsLine(58.4, n)).toBe("High 58\u00B0, 12\u00B0 above normal");
    expect(normalsLine(58.4, n, true)).toBe("High 15\u00B0, 6\u00B0 above normal");
  });

  it("hides the line when the server has no normals yet or none here", async () => {
    const realFetch = global.fetch;
    for (const status of [404, 503]) {
      global.fetch = jest.fn(async () => new Response("{}", { status })) as never;
      await expect(fetchNormals("https://radar.example", 42.33, -83.05, "2026-11-18")).resolves.toBeNull();
    }
    global.fetch = jest.fn(async (url: string) => {
      expect(url).toBe("https://radar.example/api/normals/42.33/-83.05?date=2026-11-18");
      return new Response(JSON.stringify(contract.detroit), { status: 200 });
    }) as never;
    await expect(fetchNormals("https://radar.example", 42.33, -83.05, "2026-11-18")).resolves.toEqual({
      date: "2026-11-18",
      highF: 46.9,
      lowF: 30.9,
    });
    global.fetch = realFetch;
  });
});
