import { readFileSync } from "fs";
import path from "path";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { getString } from "../../src/lib/storage";

jest.mock("../../src/lib/storage", () => ({
  getString: jest.fn((_k: string, d: string) => d),
  setString: jest.fn(),
}));

const read = (rel: string) => readFileSync(path.join(__dirname, "../../src", rel), "utf8");

/**
 * Source contracts for the in-map engine's MapLibre layers (the map itself is native, so a unit
 * test cannot draw them): same insertion point as the raster frames and instant frame switches.
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

  it("colors tiles offscreen with the SkSL palette shader", () => {
    expect(source).toContain("PACKED_SKSL");
    expect(read("components/map/inMapRadarEngine.ts")).toContain("makeShaderWithChildren");
  });
});

describe("radar screen engine wiring", () => {
  const screen = read("app/(tabs)/radar.tsx");

  it("mounts the in-map frames before the anchor whenever fast radar has packed frames", () => {
    expect(screen.indexOf("{packed ? <InMapPackedRadar")).toBeGreaterThan(0);
    expect(screen.indexOf("<InMapPackedRadar")).toBeLessThan(screen.indexOf("<WeatherAnchor />"));
    expect(screen).not.toContain("radarEngine");
  });

  it("draws the normal raster radar instead while the in-map safety net has fired", () => {
    expect(screen).toContain("onFallback={setInMapFallback}");
    expect(read("components/map/InMapPackedRadar.tsx")).toContain("decideRasterFallback(");
    // The carousels only skip packed frames while In-map is drawing them.
    expect(screen.match(/isPacked=\{inMapPacked\}/g)).toHaveLength(2);
    expect(screen).toContain("const inMapPacked = packed && !inMapFallback ? packed.isPacked : undefined;");
    expect(screen).not.toContain("PackedRadarOverlay");
  });
});

describe("in-map tile zoom and diagnostics wiring", () => {
  const radar = read("components/map/InMapPackedRadar.tsx");

  it("covers at MapLibre's rounded raster zoom, in the view hook and the cover", () => {
    expect(radar).toContain("usePackedCoverView(camera, size, zoomBounds)");
    expect(read("components/map/usePackedCoverView.ts")).toContain("Math.round(zoom)");
    expect(radar).toContain("tileCover(settled.camera, settled.screen, loc.meta.min_zoom, loc.meta.max_zoom)");
  });

  it("shows the diagnostics under the Fast radar row, only while Fast radar is on", () => {
    const settings = read("app/(tabs)/settings.tsx");
    const fast = settings.indexOf('label="Fast radar (GPU)"');
    const diag = settings.indexOf("inMapSummary.map");
    expect(fast).toBeGreaterThan(0);
    expect(diag).toBeGreaterThan(fast);
    expect(settings.slice(fast, diag)).toContain("{fastRadar ? (");
    expect(settings).not.toContain("Radar engine");
  });

  it("leaves no empty catch in the in-map path", () => {
    for (const file of ["components/map/inMapRadarEngine.ts", "lib/inMapRadarDisk.ts"]) {
      expect(read(file)).not.toMatch(/catch\s*(\([^)]*\))?\s*\{\s*\}/);
    }
  });
});

describe("fast radar has no engine choice", () => {
  it("keeps no radarEngine pref and never reads the old persisted key", () => {
    const state = useWeatherStore.getInitialState();
    expect(state).not.toHaveProperty("radarEngine");
    expect(state).not.toHaveProperty("setRadarEngine");
    expect(read("stores/useWeatherStore.ts")).not.toContain("radarEngine");
    expect(getString).not.toHaveBeenCalledWith("radarEngine", expect.anything());
  });
});
