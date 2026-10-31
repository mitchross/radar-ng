import { readFileSync } from "fs";
import path from "path";
import {
  DEFAULT_RADAR_ENGINE,
  parseRadarEngine,
  RADAR_ENGINE_OPTIONS,
} from "../../src/lib/persistedPrefs";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { setString } from "../../src/lib/storage";

jest.mock("../../src/lib/storage", () => ({
  getString: jest.fn((_k: string, d: string) => d),
  setString: jest.fn(),
}));

const read = (rel: string) => readFileSync(path.join(__dirname, "../../src", rel), "utf8");

/**
 * Source contracts for the in-map engine's MapLibre layers (the map itself is native, so a unit
 * test cannot draw them): same insertion point as the classic raster frames, instant frame
 * switches, and the Skia overlay kept for the default engine.
 */
describe("InMapPackedRadar contracts", () => {
  const source = read("components/map/InMapPackedRadar.tsx");

  it("inserts every frame layer under the weather anchor, so warnings, storm cells and the pin draw above", () => {
    expect(source).toContain('import { WEATHER_ANCHOR_ID } from "./RasterFrameCarousel"');
    expect(source).toContain("beforeId={WEATHER_ANCHOR_ID}");
    expect(source.match(/<Layer\b/g)).toHaveLength(1);
    expect(source.match(/beforeId=/g)).toHaveLength(1);
    expect(read("components/map/RasterFrameCarousel.tsx")).toContain('export const WEATHER_ANCHOR_ID = "weather-anchor"');
  });

  it("switches frames by opacity with no fade and no transition", () => {
    expect(source).toContain('"raster-opacity": spec.visible && index === currentIndex ? opacity : 0');
    expect(source).toContain('"raster-fade-duration": 0');
    expect(source).toContain('"raster-opacity-transition": { duration: 0, delay: 0 }');
  });

  it("renders a keyed array with one 512-px source per frame, not a Fragment", () => {
    const body = source.slice(source.indexOf("export function InMapFrameLayers"));
    expect(body).toContain("return slots.map(");
    expect(body).not.toContain("<>");
    expect(body).toContain("key={spec.sourceId}");
    expect(body).toContain("tileSize={PACKED_TILE_SIZE}");
    expect(body).toContain("maxzoom={spec.maxZoom}");
  });

  it("hands MapLibre file:// templates only for frames that are on disk", () => {
    const tiles = read("lib/inMapRadarTiles.ts");
    expect(tiles).toContain("visible: false");
    expect(tiles).toContain("tileUrlTemplate(rootUri, frame.layer, frame.frameKey, disk.gen)");
    expect(read("lib/inMapRadarDisk.ts")).toContain("Paths.cache.uri");
  });

  it("colors with the same SkSL shader as the Skia overlay", () => {
    expect(source).toContain("PACKED_SKSL");
    expect(read("components/map/PackedRadarOverlay.tsx")).toContain("PACKED_SKSL");
    expect(read("components/map/inMapRadarEngine.ts")).toContain("makeShaderWithChildren");
  });
});

describe("radar screen engine wiring", () => {
  const screen = read("app/(tabs)/radar.tsx");

  it("mounts the in-map frames before the anchor and keeps the Skia overlay for the other engine", () => {
    expect(screen.indexOf("<InMapPackedRadar")).toBeGreaterThan(0);
    expect(screen.indexOf("<InMapPackedRadar")).toBeLessThan(screen.indexOf("<WeatherAnchor />"));
    expect(screen).toContain("{packed && (!inMap || inMapFallback) ? <PackedRadarOverlay");
    expect(screen).toContain('radarEngine === "in-map"');
  });

  it("draws the Skia overlay instead while the in-map safety net has fired", () => {
    expect(screen).toContain("onFallback={setInMapFallback}");
    expect(read("components/map/InMapPackedRadar.tsx")).toContain("decideInMapFallback(");
  });
});

describe("in-map tile zoom and diagnostics wiring", () => {
  const radar = read("components/map/InMapPackedRadar.tsx");

  it("covers at MapLibre's rounded raster zoom, in the view hook and the cover", () => {
    expect(radar).toContain('usePackedCoverView(camera, size, zoomBounds, "round")');
    expect(radar).toContain('loc.meta.max_zoom, "round")');
    expect(read("components/map/PackedRadarOverlay.tsx")).not.toContain('"round"');
  });

  it("shows the diagnostics under the Radar engine row only for In-map", () => {
    const settings = read("app/(tabs)/settings.tsx");
    expect(settings).toContain('radarEngine === "in-map" ? (');
    expect(settings.indexOf("inMapSummary.map")).toBeGreaterThan(settings.indexOf('title="Radar engine"'));
  });

  it("leaves no empty catch in the in-map path", () => {
    for (const file of ["components/map/inMapRadarEngine.ts", "lib/inMapRadarDisk.ts"]) {
      expect(read(file)).not.toMatch(/catch\s*(\([^)]*\))?\s*\{\s*\}/);
    }
  });
});

describe("radar engine setting", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    useWeatherStore.setState(useWeatherStore.getInitialState());
  });

  it("defaults to Classic fast until a device test decides otherwise", () => {
    expect(DEFAULT_RADAR_ENGINE).toBe("skia");
    expect(useWeatherStore.getState().radarEngine).toBe("skia");
    expect(RADAR_ENGINE_OPTIONS[0]).toEqual({ value: "skia", label: "Classic fast" });
    expect(RADAR_ENGINE_OPTIONS.find((o) => o.value === "in-map")?.label).toBe("In-map");
  });

  it("persists the choice under radarEngine", () => {
    useWeatherStore.getState().setRadarEngine("in-map");
    expect(setString).toHaveBeenCalledWith("radarEngine", "in-map");
    expect(useWeatherStore.getState().radarEngine).toBe("in-map");
  });

  it("falls back to the default on a stale stored value", () => {
    expect(parseRadarEngine("in-map")).toBe("in-map");
    expect(parseRadarEngine("webgl")).toBe("skia");
    expect(parseRadarEngine("")).toBe("skia");
  });

  it("shows the picker next to Fast radar, only while Fast radar is on", () => {
    const settings = read("app/(tabs)/settings.tsx");
    const fast = settings.indexOf('label="Fast radar (GPU)"');
    const engine = settings.indexOf('title="Radar engine"');
    expect(fast).toBeGreaterThan(0);
    expect(engine).toBeGreaterThan(fast);
    expect(settings.slice(fast, engine)).toContain("{fastRadar ? (");
    expect(settings).toContain("keeps warnings, storm cells and your pin above the radar");
  });
});
