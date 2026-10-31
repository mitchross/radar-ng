import { act, renderHook, waitFor } from "@testing-library/react-native";
import { clearInMapCache, createInMapEngine, type EngineFrame, type EngineInput } from "../../src/components/map/inMapRadarEngine";
import { useInMapSummary } from "../../src/hooks/useInMapStats";
import { inMapStats, recordInMapTile, resetInMapStats } from "../../src/lib/inMapDiag";
import * as disk from "../../src/lib/inMapRadarDisk";
import type { PackedFrameLocation } from "../../src/lib/packed";

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 1, 2, 3, 4]);

jest.mock("@shopify/react-native-skia", () => {
  const image = {
    makeNonTextureImage: () => ({ encodeToBytes: () => new Uint8Array([0x89, 0x50, 0x4e, 0x47, 1, 2, 3, 4]), dispose: () => {} }),
    encodeToBytes: () => new Uint8Array([0x89, 0x50, 0x4e, 0x47, 1, 2, 3, 4]),
    dispose: () => {},
  };
  const surface = {
    getCanvas: () => ({ clear: () => {}, drawRect: () => {} }),
    flush: () => {},
    makeImageSnapshot: () => image,
  };
  return {
    ImageFormat: { PNG: 4 },
    Skia: {
      Surface: { MakeOffscreen: () => surface, Make: () => surface },
      Paint: () => ({ setAntiAlias: () => {}, setShader: () => {} }),
      Color: () => 0,
      XYWHRect: () => ({}),
    },
  };
});

jest.mock("../../src/components/map/packedGpu", () => {
  const entry = { state: "ready", height: 1 << 20, handle: { shader: {} } };
  return {
    CACHE_BUDGET_BYTES: 192 * 1024 * 1024,
    atlasBytes: () => 1,
    tiles: { get: () => entry, subscribe: () => () => {}, setWanted: () => {}, touch: () => {} },
  };
});

jest.mock("../../src/lib/inMapRadarDisk", () => ({
  writeTile: jest.fn(async () => {}),
  copyTile: jest.fn(async () => true),
  verifyTile: jest.fn(() => ({ uri: "file:///cache/radar-inmap/radar/f/1/6/10/20.png", exists: true })),
  deleteRelative: jest.fn(),
  listFrames: jest.fn(() => []),
  wipeAll: jest.fn(),
}));

const loc = (bucket: string): PackedFrameLocation =>
  ({
    layer: "radar",
    bucket,
    meta: { min_zoom: 5, max_zoom: 9, max_slots: 3, cols: 1, buckets: [{ id: bucket, slots: [0, 1, 2] }] },
    lutKey: "k",
    channel: 0,
    x0: 0,
    y0: 0,
  }) as unknown as PackedFrameLocation;

const frame = (name: string): EngineFrame => ({
  loc: loc(name),
  dir: `radar/${name}`,
  frameKey: name,
  lut: {} as EngineFrame["lut"],
  urlFor: (t) => `https://atlas/${name}/${t.z}/${t.x}/${t.y}`,
});

const cover = new Map([["5|9", [{ z: 6, x: 10, y: 20 }]]]);
const input = (names: string[], keep: string[] = names): EngineInput => ({
  effect: { makeShaderWithChildren: () => ({}) } as unknown as EngineInput["effect"],
  frames: names.map(frame),
  allDirs: new Set(keep.map((n) => `radar/${n}`)),
  covers: cover,
});

// 3 x 3 tiles per frame: the cover tile and its ring.
const TILES_PER_FRAME = 9;

const summary = () => renderHook(() => useInMapSummary());

describe("In-map diagnostics in Settings", () => {
  beforeEach(() => {
    clearInMapCache();
    jest.clearAllMocks();
  });

  it("repaints a Settings screen that is mounted while the engine works", async () => {
    const view = await summary();
    expect(view.result.current[0]).toBe("In-map: 0 frames · 0 tiles · 0 KB · surface ?");
    const engine = createInMapEngine(() => {});
    engine.update(input(["a", "b"]));
    await waitFor(() => expect(view.result.current[0]).toMatch(/^In-map: 2 frames · 18 tiles · 0 KB · GPU$/));
    engine.stop();
  });

  it("shows current counts on a screen that mounts after the work", async () => {
    const engine = createInMapEngine(() => {});
    engine.update(input(["a"]));
    await waitFor(() => expect(inMapStats.framesCommitted).toBe(1));
    engine.stop();
    const late = await summary();
    expect(late.result.current[0]).toBe(`In-map: 1 frames · ${TILES_PER_FRAME} tiles · 0 KB · GPU`);
  });

  it("counts every written tile and byte, not only when a frame commits", async () => {
    const view = await summary();
    expect(disk.writeTile).not.toHaveBeenCalled();
    await act(async () => recordInMapTile(2048));
    await waitFor(() => expect(view.result.current[0]).toContain("1 tiles · 2 KB"));
  });

  it("counts the frames in the disk index: a frame that left the manifest drops out", async () => {
    const engine = createInMapEngine(() => {});
    engine.update(input(["a", "b"]));
    await waitFor(() => expect(inMapStats.framesCommitted).toBe(2));
    engine.update({ ...input(["a"], ["a"]) });
    await waitFor(() => expect(inMapStats.framesCommitted).toBe(1), { timeout: 6000 });
    engine.stop();
  }, 10_000);

  it("resets on a Classic-fast switch but keeps reporting the surface", async () => {
    const engine = createInMapEngine(() => {});
    engine.update(input(["a"]));
    const view = await summary();
    await waitFor(() => expect(view.result.current[0]).toContain("1 frames"));
    engine.stop();
    await act(async () => clearInMapCache());
    await waitFor(() => expect(view.result.current[0]).toBe("In-map: 0 frames · 0 tiles · 0 KB · GPU"));
    expect(disk.wipeAll).toHaveBeenCalled();
    expect(inMapStats.errorCount).toBe(0);
  });

  it("does not reset the stats when the app only reads them", async () => {
    recordInMapTile(10);
    await summary();
    expect(inMapStats.tilesWritten).toBe(1);
    resetInMapStats();
  });
});
