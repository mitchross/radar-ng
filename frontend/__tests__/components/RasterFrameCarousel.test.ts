import { readFileSync } from "fs";
import path from "path";

const read = (rel: string) => readFileSync(path.join(__dirname, "../../src", rel), "utf8");

/**
 * Source contracts for the frame-carousel invariants. The overlay's crash
 * safety on iOS depends on a CONSTANT child count with keyed replace-in-place
 * — these tests pin the structural properties a refactor must not lose.
 */
describe("RasterFrameCarousel contracts", () => {
  const source = read("components/map/RasterFrameCarousel.tsx");

  it("renders slots as a keyed array, not a Fragment", () => {
    expect(source).toContain("return slots.map(");
    expect(source).not.toContain("<>");
  });

  it("never conditionally nulls a slot (constant child count)", () => {
    const mapBody = source.slice(source.indexOf("return slots.map("));
    expect(mapBody).not.toMatch(/if\s*\(.*\)\s*return null/);
    expect(mapBody).toContain("?? frames[clampedIndex]");
  });

  it("swaps frames by opacity with zero fade, source id unique per slot assignment", () => {
    expect(source).toContain('"raster-opacity": isVisible ? opacity : 0');
    expect(source).toContain('"raster-fade-duration": 0');
    expect(source).toContain("-src-${slot}-${variant}-${frame.path}");
    expect(source).toContain("key={sourceId}");
  });

  it("takes the slot count from the shared CAROUSEL_WINDOW constant", () => {
    expect(source).toContain('from "../../lib/radarCarousel"');
    expect(source).toContain("assignSlots(clampedIndex, start, end, CAROUSEL_WINDOW)");
    expect(source).toContain("clampWindow(playbackWindow, frames.length)");
  });
});

describe("raster overlays share the carousel", () => {
  it.each(["components/map/RadarOverlay.tsx", "components/map/WeatherLayerOverlay.tsx"])(
    "%s renders through RasterFrameCarousel and reads the store playback window",
    (rel) => {
      const source = read(rel);
      expect(source).toContain("<RasterFrameCarousel");
      expect(source).not.toContain("<RasterSource");
      expect(source).toContain("s.playbackWindow");
    },
  );

  it("RadarOverlay keeps the server-coverage zoom caps", () => {
    const source = read("components/map/RadarOverlay.tsx");
    expect(source).toContain("SOURCE_MAX_ZOOM");
    expect(source).toContain("SOURCE_MIN_ZOOM");
    expect(source).toContain("radarVisible ? radarOpacity : 0");
  });
});

describe("map overlays keep a constant native child count", () => {
  it.each([
    "components/map/AlertPolygon.tsx",
    "components/map/LightningOverlay.tsx",
    "components/map/StormCellsOverlay.tsx",
    "components/map/TropicalOverlay.tsx",
  ])("%s always mounts its GeoJSONSource (empty collection, never null)", (rel) => {
    const source = read(rel);
    expect(source).toContain("EMPTY_FEATURE_COLLECTION");
    // Only the exported component body — private helpers may return null.
    const start = source.indexOf("export function");
    const body = source.slice(start, source.indexOf("\n}\n", start));
    expect(body).not.toMatch(/return null/);
    expect(body).toContain("<GeoJSONSource");
  });

  it("the eyedropper pin hides instead of unmounting", () => {
    const source = read("components/inspector/Eyedropper.tsx");
    expect(source).not.toMatch(/if \(!pinned\) return null/);
    expect(source).toContain("hidden ? styles.hidden : null");
  });

  it("the radar screen mounts the data overlays unconditionally", () => {
    const source = read("app/(tabs)/radar.tsx");
    expect(source).toContain("<StormCellsOverlay />");
    expect(source).toContain("<LightningOverlay />");
    expect(source).not.toContain("extrasVisible &&");
    expect(source).not.toContain("pinned &&");
  });
});
