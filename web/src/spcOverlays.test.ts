import { afterEach, expect, it, vi } from "vitest";
import { Overlays } from "./overlays";
import type { Map } from "maplibre-gl";

const now = Date.parse("2026-11-08T00:00:00Z");
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it("installs independently toggled outlines and tags, fetches our endpoints and removes expired cached products on failure", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(now);
  vi.stubGlobal("window", globalThis);
  const layers: any[] = [];
  const sources: Record<string, any> = {};
  const map = {
    addSource: (id: string, source: any) => { sources[id] = { data: source.data, setData(data: any) { this.data = data; } }; },
    addLayer: (layer: any) => layers.push(layer),
    setPaintProperty: vi.fn(),
    getLayer: (id: string) => layers.find((l) => l.id === id),
    getSource: (id: string) => sources[id],
    getStyle: () => ({ layers }),
    setLayoutProperty: (id: string, prop: string, value: string) => { layers.find((l) => l.id === id).layout[prop] = value; },
  };
  const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ type: "FeatureCollection", features: [{ type: "Feature", geometry: { type: "Polygon", coordinates: [] }, properties: { expires: "2026-11-08T00:01:00Z" } }] }) });
  vi.stubGlobal("fetch", fetcher);
  const overlays = new Overlays(map as unknown as Map, new Set(["watches"]));
  overlays.install();
  expect(map.getLayer("ov-watches-line").layout.visibility).toBe("visible");
  expect(map.getLayer("ov-discussions-line").layout.visibility).toBe("none");
  expect(map.getLayer("ov-watches-labels").layout["text-field"]).toContain("TOR WATCH");
  expect(overlays.clickable()).toContain("ov-watches-hit");
  await overlays.refresh("watches");
  expect(fetcher).toHaveBeenCalledWith("/api/spc/watches");
  expect(sources["ov-watches"].data.features).toHaveLength(1);
  await overlays.refresh("discussions");
  expect(fetcher).toHaveBeenCalledWith("/api/spc/discussions");
  overlays.setEnabled("watches", false);
  expect(map.getLayer("ov-watches-line").layout.visibility).toBe("none");
  vi.setSystemTime(now + 60_000);
  fetcher.mockRejectedValue(new Error("offline"));
  await overlays.refresh("watches");
  expect(sources["ov-watches"].data.features).toEqual([]);
});
