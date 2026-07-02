import { readFileSync } from "fs";
import path from "path";

/**
 * Source contracts for the frame-carousel invariants. The overlay's crash
 * safety on iOS depends on a CONSTANT child count with keyed replace-in-place
 * — these tests pin the structural properties a refactor must not lose.
 */
describe("RadarOverlay carousel contracts", () => {
  const source = readFileSync(
    path.join(__dirname, "../../src/components/map/RadarOverlay.tsx"),
    "utf8",
  );

  it("renders slots as a keyed array, not a Fragment", () => {
    expect(source).toContain("return slots.map(");
    expect(source).not.toContain("<>");
  });

  it("never conditionally nulls a slot (constant child count)", () => {
    // The map callback must not early-return null — a missing frame falls
    // back to the current frame instead of dropping the child.
    const mapBody = source.slice(source.indexOf("return slots.map("));
    expect(mapBody).not.toMatch(/if\s*\(.*\)\s*return null/);
    expect(mapBody).toContain("?? frames[clampedIndex]");
  });

  it("swaps frames by opacity, with source id unique per slot assignment", () => {
    expect(source).toContain('"raster-opacity": isVisible && radarVisible ? radarOpacity : 0');
    expect(source).toContain("radar-src-${slot}-");
  });

  it("keeps the server-coverage zoom caps", () => {
    expect(source).toContain("SOURCE_MAX_ZOOM");
    expect(source).toContain("SOURCE_MIN_ZOOM");
  });

  it("derives assignments from the shared carousel helper", () => {
    expect(source).toContain('from "../../lib/radarCarousel"');
    expect(source).toContain("assignSlots(");
    expect(source).toContain("clampWindow(");
  });
});
