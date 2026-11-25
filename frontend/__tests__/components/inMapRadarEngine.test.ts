import type { SkRuntimeEffect } from "@shopify/react-native-skia";

const mockDeleteRelative = jest.fn();
const mockListFrames = jest.fn();

jest.mock("@shopify/react-native-skia", () => ({
  ImageFormat: { PNG: 0 },
  Skia: {
    Surface: {
      MakeOffscreen: jest.fn(() => null),
      Make: jest.fn(() => null),
    },
    Paint: jest.fn(() => ({ setShader: jest.fn() })),
  },
}));

jest.mock("../../src/lib/inMapRadarDisk", () => ({
  copyTile: jest.fn(),
  deleteRelative: (...args: unknown[]) => mockDeleteRelative(...args),
  listFrames: (...args: unknown[]) => mockListFrames(...args),
  verifyTile: jest.fn(() => true),
  wipeAll: jest.fn(),
  writeTile: jest.fn(),
}));

jest.mock("../../src/components/map/packedGpu", () => ({
  atlasBytes: jest.fn(() => 0),
  CACHE_BUDGET_BYTES: 192 * 1024 * 1024,
  tiles: {
    get: jest.fn(() => undefined),
    load: jest.fn(),
    subscribe: jest.fn(() => () => {}),
  },
}));

import { createInMapEngine } from "../../src/components/map/inMapRadarEngine";

describe("createInMapEngine sweep scheduling", () => {
  beforeEach(() => {
    jest.useFakeTimers();
    mockDeleteRelative.mockReset();
    mockListFrames.mockReset();
  });

  afterEach(() => {
    jest.useRealTimers();
  });

  it("fires the 2500 ms disk sweep during steady 10 FPS playback updates", async () => {
    mockListFrames.mockReturnValue([{ layer: "radar", frameKey: "expired-frame", gens: [1] }]);
    const engine = createInMapEngine(() => {});
    const covers = new Map();
    const input = {
      effect: {} as SkRuntimeEffect,
      frames: [],
      allDirs: new Set(["radar/current-frame"]),
      covers,
    };

    try {
      // Simulate steady 10 FPS playback (update every 100 ms for 3 seconds).
      for (let tick = 0; tick < 30; tick++) {
        engine.update(input);
        await Promise.resolve();
        jest.advanceTimersByTime(100);
      }

      expect(mockDeleteRelative).toHaveBeenCalledTimes(1);
      expect(mockDeleteRelative).toHaveBeenCalledWith(["radar/expired-frame"]);
    } finally {
      engine.stop();
    }
  });
});
