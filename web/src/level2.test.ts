import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "fs";
import path from "path";
import type { Map as MLMap } from "maplibre-gl";
import { Level2Layer } from "./level2";
import type { Level2Frame, Level2Snapshot, Level2View } from "./shared/level2";

function makeFrame(pathKey: string): Level2Frame {
  return {
    time: 1_760_000_000,
    timestamp: "2026-10-15T22:00:00Z",
    scanEndMs: 1_760_000_000_000,
    scanStart: null,
    path: pathKey,
    sweepId: pathKey,
    volumeId: null,
    stationId: "KGRR",
    stationName: "Grand Rapids",
    stationLat: 42.89,
    stationLon: -85.54,
    elevationDeg: 0.5,
    antennaAltitudeMMsl: null,
    qcProfile: null,
    decoder: null,
    dualPol: { rho: null, zdr: null },
    sourceCount: 1,
    samplerVersion: "1",
    missingSectors: 0,
  };
}

describe("Level2Layer.repaint", () => {
  it("updates the shown sweep layer opacity when the slider changes while paused", () => {
    const layers = new Set<string>();
    const paintCalls: Array<[string, string, number]> = [];
    const map = {
      on: vi.fn(),
      addSource: vi.fn(),
      addLayer: vi.fn((spec: { id: string }) => layers.add(spec.id)),
      getLayer: vi.fn((id: string) => (layers.has(id) ? { id } : undefined)),
      getSource: vi.fn(() => undefined),
      moveLayer: vi.fn(),
      removeLayer: vi.fn((id: string) => layers.delete(id)),
      removeSource: vi.fn(),
      setPaintProperty: vi.fn((id: string, prop: string, val: number) => {
        paintCalls.push([id, prop, val]);
      }),
    } as unknown as MLMap;

    const frameA = makeFrame("sweep-a");
    const frameB = makeFrame("sweep-b");
    const view: Level2View = {
      viewId: "v1",
      revision: "1",
      stationId: "KGRR",
      stationName: "Grand Rapids",
      stationLat: 42.89,
      stationLon: -85.54,
      sourceZoom: 8,
      palette: "classic",
      requiredTiles: [{ z: 8, x: 66, y: 94 }],
      tileTemplate: "/level2/tiles/{station}/{sweep}/{z}/{x}/{y}.png",
      expiresAtMs: Date.now() + 60_000,
      frames: [frameA, frameB],
    };
    const snap: Level2Snapshot = {
      phase: "ready",
      key: "v1.1|1|1",
      view,
      opacity: 0.8,
      unavailable: false,
      reason: null,
    };

    let currentOpacity = 0.75;
    const origLocation = globalThis.location;
    Object.defineProperty(globalThis, "location", {
      value: { origin: "https://radar.vanillax.me" },
      configurable: true,
    });

    try {
      const layer = new Level2Layer(map, {
        palette: () => "classic",
        active: () => true,
        opacity: () => currentOpacity,
        changed: vi.fn(),
        beforeId: () => "weather-anchor",
      });
      vi.spyOn(layer.session, "snapshot").mockReturnValue(snap);

      layer.show(frameA);
      paintCalls.length = 0;

      // Drag opacity slider while paused on frameA.
      currentOpacity = 0.4;
      layer.repaint();

      expect(paintCalls).toEqual([
        ["l2-v1.1-0", "raster-opacity", expect.closeTo(0.32, 6)],
        ["l2-v1.1-1", "raster-opacity", 0],
      ]);
    } finally {
      Object.defineProperty(globalThis, "location", {
        value: origLocation,
        configurable: true,
      });
    }
  });

  it("wires the web opacity slider handler to level2.repaint()", () => {
    const mainSrc = readFileSync(path.join(__dirname, "main.ts"), "utf8");
    const handlerStart = mainSrc.indexOf('if (el.id !== "opacity") return;');
    expect(handlerStart).toBeGreaterThan(0);
    const snippet = mainSrc.slice(handlerStart, handlerStart + 320);
    expect(snippet).toContain("level2.repaint()");
  });
});
