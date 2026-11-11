import fs from "fs";
import path from "path";
import { QueryClient } from "@tanstack/react-query";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import type { ReplayCase } from "../../src/lib/replayApi";
import { replayManifestKey, replayWarningsKey, replayCatalogueKey, replayGuard } from "../../src/lib/replayClient";
import { exitReplay } from "../../src/lib/replayControl";
import { replayRadarFrames, layerForCase, liveFramesReady } from "../../src/lib/replayState";
import { buildReplayTileUrl } from "../../src/lib/tileUrl";
import { replayTimelineHeader } from "../../src/lib/replayTimeline";
import { LIVE_SOURCE } from "../../src/lib/replaySource";
import type { RadarFrame } from "../../src/types/weather";

jest.mock("../../src/lib/storage", () => ({ getString: jest.fn((_k: string, d: string) => d), setString: jest.fn() }));

const fixture = require("../../../web/src/shared/replayApi.fixture.json");
const rolling: ReplayCase = fixture.catalogue.cases[0];
const frame = (time: number): RadarFrame => ({ time, timestamp: new Date(time * 1000).toISOString(), path: String(time) });

beforeEach(() => {
  useWeatherStore.setState(useWeatherStore.getInitialState());
});

describe("source switching", () => {
  it("entering a replay clears frames, stops playback and fits the case bounds", () => {
    useWeatherStore.setState({ frames: [frame(1), frame(2)], currentFrameIndex: 1, isPlaying: true, activeLayer: "wind" });
    useWeatherStore.getState().enterReplay(rolling);
    const s = useWeatherStore.getState();
    expect(s.source).toEqual({ kind: "replay", caseId: "rolling-fork-2023", revision: "contract-1" });
    expect(s.frames).toEqual([]);
    expect(s.currentFrameIndex).toBe(-1);
    expect(s.isPlaying).toBe(false);
    expect(s.activeLayer).toBe("radar");
    expect(s.focusBounds).toEqual(rolling.bounds);
  });

  it("aborts in-flight replay requests on switch", () => {
    const t = replayGuard.ticket();
    useWeatherStore.getState().enterReplay(rolling);
    expect(t.signal.aborted).toBe(true);
    const t2 = replayGuard.ticket();
    useWeatherStore.getState().returnToLive();
    expect(t2.signal.aborted).toBe(true);
  });

  it("drops a late timeline from a source that is no longer active", () => {
    useWeatherStore.getState().enterReplay(rolling);
    useWeatherStore.getState().setFrameTimeline([frame(5)], 0, LIVE_SOURCE);
    expect(useWeatherStore.getState().frames).toEqual([]);
  });

  it("keeps the UTC playhead when the layer's frames change in replay", () => {
    useWeatherStore.getState().enterReplay(rolling);
    useWeatherStore.getState().setFrameTimeline([frame(10), frame(20), frame(30)], 0);
    useWeatherStore.getState().setCurrentFrameIndex(2);
    useWeatherStore.getState().setFrameTimeline([frame(10), frame(20), frame(30), frame(40)], 0);
    expect(useWeatherStore.getState().currentFrameIndex).toBe(2);
  });

  it("refuses a layer the case does not offer", () => {
    useWeatherStore.getState().enterReplay({ ...rolling, layers: ["radar"] });
    useWeatherStore.getState().setActiveLayer("wind");
    expect(useWeatherStore.getState().activeLayer).toBe("radar");
  });

  it("return to live clears replay state and marks the return time", () => {
    useWeatherStore.getState().enterReplay(rolling);
    useWeatherStore.getState().setFrameTimeline([frame(10)], 0);
    useWeatherStore.getState().returnToLive();
    const s = useWeatherStore.getState();
    expect(s.source).toEqual(LIVE_SOURCE);
    expect(s.replayCase).toBeNull();
    expect(s.frames).toEqual([]);
    expect(s.liveReturnedAt).not.toBeNull();
  });
});

describe("replay data wiring", () => {
  it("builds one frame per base frame with no merging", () => {
    const frames = replayRadarFrames(fixture.manifest, "radar");
    expect(frames).toHaveLength(fixture.manifest.layers.radar.frames.length);
    expect(frames.every((f) => f.source === undefined || typeof f.source === "string")).toBe(true);
  });

  it("keeps the current layer only if the case has it", () => {
    expect(layerForCase("hail-size", rolling.layers)).toBe("hail-size");
    expect(layerForCase("wind", rolling.layers)).toBe("radar");
    expect(layerForCase("hail-size", ["radar"])).toBe("radar");
  });

  it("builds a classic-palette case tile URL", () => {
    const url = buildReplayTileUrl("https://s", fixture.manifest.tile_url_template, "radar", "2023-03-24T23:00:00Z");
    expect(url.startsWith("https://s/replays/rolling-fork-2023/contract-1/tiles/radar/classic/")).toBe(true);
  });

  it("labels the playhead with absolute date, time and zone, never Now", () => {
    const h = replayTimelineHeader(frame(Date.parse("2023-03-25T01:00:00Z") / 1000), "Radar", "America/Chicago");
    expect(h.dateLabel).toMatch(/2023/);
    expect(h.dateLabel).toMatch(/CDT|CST/);
    expect(`${h.title}${h.dateLabel}`).not.toMatch(/Now/);
  });
});

describe("query keys", () => {
  it("replay keys carry the source key and never the live manifest key", () => {
    const a = replayManifestKey("https://s", { kind: "replay", caseId: "c", revision: "1" });
    const b = replayManifestKey("https://s", { kind: "replay", caseId: "c", revision: "2" });
    expect(a).not.toEqual(b);
    expect(a[0]).not.toBe("manifest");
    expect(JSON.stringify(a)).toContain("replay:c:1");
    expect(JSON.stringify(replayWarningsKey("https://s", { kind: "replay", caseId: "c", revision: "1" }))).toContain("replay:c:1");
    expect(replayCatalogueKey("https://s")[0]).toBe("replay-catalogue");
  });
});

describe("Return to live", () => {
  it("clears replay caches and force-refetches the live manifest", async () => {
    useWeatherStore.setState({ serverUrl: "https://s" });
    useWeatherStore.getState().enterReplay(rolling);
    const qc = new QueryClient();
    const invalidate = jest.spyOn(qc, "invalidateQueries").mockResolvedValue();
    const remove = jest.spyOn(qc, "removeQueries");
    await exitReplay(qc);
    expect(useWeatherStore.getState().source).toEqual(LIVE_SOURCE);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["manifest", "https://s"], refetchType: "all" });
    expect(remove).toHaveBeenCalledWith({ queryKey: ["replay-manifest"] });
  });

  it("holds live frames back until a manifest newer than the return arrives", () => {
    expect(liveFramesReady(100, 200)).toBe(false);
    expect(liveFramesReady(300, 200)).toBe(true);
    expect(liveFramesReady(0, null)).toBe(true);
  });
});

describe("notification path", () => {
  it("never reads replay modules", () => {
    const src = path.join(__dirname, "../../src");
    for (const f of ["hooks/useAlertNotifications.ts", "hooks/useAlerts.ts", "lib/alertNotificationScheduler.ts", "lib/alertLifecycle.ts", "tasks/rainAlertsTask.ts"]) {
      const text = fs.readFileSync(path.join(src, f), "utf8");
      expect(text).not.toMatch(/replay/i);
    }
  });
});
