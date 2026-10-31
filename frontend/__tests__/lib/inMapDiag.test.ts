import {
  decideInMapFallback,
  diagEventDue,
  errorMessage,
  formatInMapSummary,
  genSuffix,
  hasPngSignature,
  IN_MAP_BLANK_MS,
  inMapDiagAttributes,
  inMapStats,
  patchInMapStats,
  recordInMapError,
  resetInMapStats,
  type InMapStage,
} from "../../src/lib/inMapDiag";
import { privacySafeAttributes } from "../../src/lib/telemetryPrivacy";

const base = { canRender: true, hasCover: true, waitedMs: 0, committedFrames: 0, verifyFailed: false };

describe("decideInMapFallback", () => {
  it("draws in-map while the first frame is still being written", () => {
    expect(decideInMapFallback({ ...base, waitedMs: IN_MAP_BLANK_MS - 1 })).toBeNull();
  });

  it("falls back once 8 s pass with a cover and no committed frame", () => {
    expect(IN_MAP_BLANK_MS).toBe(8_000);
    expect(decideInMapFallback({ ...base, waitedMs: IN_MAP_BLANK_MS })).toMatch(/no frame/);
  });

  it("does not time out without tiles in view or once a frame has committed", () => {
    expect(decideInMapFallback({ ...base, hasCover: false, waitedMs: 60_000 })).toBeNull();
    expect(decideInMapFallback({ ...base, waitedMs: 60_000, committedFrames: 1 })).toBeNull();
  });

  it("falls back at once when a committed frame has no file, and when there is no surface", () => {
    expect(decideInMapFallback({ ...base, committedFrames: 3, verifyFailed: true })).toMatch(/verify/);
    expect(decideInMapFallback({ ...base, canRender: false })).toMatch(/surface/);
  });

  it("switches back to in-map once a frame commits and verifies", () => {
    const stuck = { ...base, waitedMs: 20_000 };
    expect(decideInMapFallback(stuck)).not.toBeNull();
    expect(decideInMapFallback({ ...stuck, committedFrames: 1 })).toBeNull();
  });
});

describe("stats and summary", () => {
  beforeEach(resetInMapStats);

  it("records the stage and a short single-line message, and counts errors", () => {
    recordInMapError("encode", new Error("empty"));
    recordInMapError("write", "failed to write file:///var/mobile/Caches/radar-inmap/a/1/2/3.png now\nplease");
    expect(inMapStats.errorCount).toBe(2);
    expect(inMapStats.lastError).toEqual({ stage: "write", message: "failed to write <uri> now please" });
    expect(errorMessage("x".repeat(500))).toHaveLength(120);
    expect(errorMessage({})).toBe("unknown");
  });

  it("reads like the line Mitch reports", () => {
    patchInMapStats({ surface: "gpu", framesCommitted: 8, tilesWritten: 212, bytesWritten: 3 * 1024 * 1024 });
    recordInMapError("encode", new Error("empty"));
    const [first, second] = formatInMapSummary(inMapStats);
    expect(first).toBe("In-map: 8 frames · 212 tiles · 3.0 MB · GPU");
    expect(second).toContain("last error: encode empty");
  });

  it("shows zoom against written zoom, PNG check, first file and fallback", () => {
    patchInMapStats({ mapZoom: 5.7, writtenZoom: 6, lastPngBytes: 812, lastPngSignature: true, firstTileExists: false, fallback: "no frame in 8s" });
    const second = formatInMapSummary(inMapStats)[1];
    expect(second).toContain("fallback: no frame in 8s");
    expect(second).toContain("no errors");
    expect(second).toContain("zoom 5.7 / z6");
    expect(second).toContain("png 812 B ok");
    expect(second).toContain("file missing");
  });

  it("recognises the PNG signature", () => {
    expect(hasPngSignature(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d]))).toBe(true);
    expect(hasPngSignature(new Uint8Array([0x89, 0x50]))).toBe(false);
    expect(hasPngSignature(new Uint8Array())).toBe(false);
    expect(hasPngSignature(new Uint8Array([0xff, 0xd8, 0xff, 0xe0]))).toBe(false);
  });
});

describe("telemetry", () => {
  beforeEach(resetInMapStats);

  it("keeps the tile directory without the tile's z/x/y", () => {
    expect(genSuffix("file:///var/mobile/Library/Caches/radar-inmap/radar/b.v1.s0.ab12/3/6/22/41.png")).toBe(
      "radar-inmap/radar/b.v1.s0.ab12/3",
    );
    expect(genSuffix(null)).toBeNull();
    expect(genSuffix("file:///elsewhere/1.png")).toBeNull();
  });

  it("sends counts, zoom and errors that the privacy filter leaves alone", () => {
    patchInMapStats({
      surface: "gpu",
      framesCommitted: 2,
      tilesWritten: 40,
      mapZoom: 5.66,
      writtenZoom: 6,
      firstTileUri: "file:///var/mobile/Caches/radar-inmap/radar/k/1/6/20/24.png",
      firstTileExists: true,
    });
    recordInMapError("verify", "tile file missing after write");
    const attrs = inMapDiagAttributes(inMapStats, "periodic");
    expect(attrs["inmap.zoom"]).toBe(5.7);
    expect(attrs["inmap.err_stage"]).toBe("verify");
    expect(attrs["inmap.tile_dir"]).toBe("radar-inmap/radar/k/1");
    expect(privacySafeAttributes(attrs)).toEqual(attrs);
    expect(JSON.stringify(attrs)).not.toMatch(/\/20\/24|file:/);
  });

  it("sends at most one event a minute, plus one per new error stage", () => {
    const seen = new Set<InMapStage>();
    expect(diagEventDue(null, 0, null, seen)).toBe(true);
    expect(diagEventDue(0, 30_000, null, seen)).toBe(false);
    expect(diagEventDue(0, 60_000, null, seen)).toBe(true);
    expect(diagEventDue(60_000, 61_000, "encode", seen)).toBe(true);
    expect(diagEventDue(61_000, 62_000, "encode", seen)).toBe(false);
    expect(diagEventDue(61_000, 62_000, "write", seen)).toBe(true);
  });
});

describe("fallback edge cases", () => {
  it("names the 8 s reason exactly as Settings and telemetry show it", () => {
    expect(decideInMapFallback({ ...base, waitedMs: IN_MAP_BLANK_MS })).toBe("no frame in 8s");
  });

  it("checks the surface first, then the verify result, then the timeout", () => {
    const everything = { canRender: false, hasCover: true, waitedMs: 60_000, committedFrames: 0, verifyFailed: true };
    expect(decideInMapFallback(everything)).toBe("no offscreen surface");
    expect(decideInMapFallback({ ...everything, canRender: true })).toBe("verify: no file on disk");
    expect(decideInMapFallback({ ...everything, canRender: true, verifyFailed: false })).toBe("no frame in 8s");
  });

  it("falls back on a missing file or surface even before any tiles are in view", () => {
    expect(decideInMapFallback({ ...base, hasCover: false, verifyFailed: true })).toMatch(/verify/);
    expect(decideInMapFallback({ ...base, hasCover: false, canRender: false })).toMatch(/surface/);
  });

  it("draws in-map when the cover has just appeared, with or without frames", () => {
    expect(decideInMapFallback({ ...base, waitedMs: 0 })).toBeNull();
    expect(decideInMapFallback({ ...base, waitedMs: 0, committedFrames: 4 })).toBeNull();
  });

  it("stays on in-map after the 8 s once any frame is committed", () => {
    expect(decideInMapFallback({ ...base, waitedMs: IN_MAP_BLANK_MS * 10, committedFrames: 1 })).toBeNull();
  });
});

describe("summary with an error and a fallback", () => {
  beforeEach(resetInMapStats);

  it("puts the fallback first on the second line, then the error stage and message", () => {
    patchInMapStats({ surface: "cpu", fallback: "no frame in 8s" });
    recordInMapError("atlas-load", new Error("atlas download failed"));
    const [first, second] = formatInMapSummary(inMapStats);
    expect(first).toBe("In-map: 0 frames · 0 tiles · 0 KB · CPU");
    expect(second).toBe("fallback: no frame in 8s · last error: atlas-load atlas download failed");
  });

  it("shows the latest error stage when several fail, and no errors after a reset", () => {
    recordInMapError("encode", "empty");
    recordInMapError("write", "disk full");
    expect(formatInMapSummary(inMapStats)[1]).toBe("last error: write disk full");
    resetInMapStats();
    expect(formatInMapSummary(inMapStats)[1]).toBe("no errors");
  });

  it("reports no surface and a bad PNG next to the error", () => {
    patchInMapStats({ surface: "none", lastPngBytes: 0, lastPngSignature: false });
    recordInMapError("surface", "no surface could be made");
    const [first, second] = formatInMapSummary(inMapStats);
    expect(first).toContain("no surface");
    expect(second).toBe("last error: surface no surface could be made · png 0 B bad");
  });

  it("sends the fallback and the error stage to telemetry and drops them on reset", () => {
    patchInMapStats({ fallback: "verify: no file on disk" });
    recordInMapError("verify", "tile file missing after write");
    const attrs = inMapDiagAttributes(inMapStats, "error");
    expect(attrs["inmap.fallback"]).toBe("verify: no file on disk");
    expect(attrs["inmap.err_stage"]).toBe("verify");
    expect(attrs["inmap.errors"]).toBe(1);
    resetInMapStats();
    expect(inMapDiagAttributes(inMapStats, "periodic")["inmap.fallback"]).toBeUndefined();
  });
});
