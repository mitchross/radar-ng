import { readFileSync } from "fs";
import path from "path";

describe("WeatherMap", () => {
  it("does not mount MapLibre native UserLocation", () => {
    const source = readFileSync(
      path.join(__dirname, "../../src/components/map/WeatherMap.tsx"),
      "utf8",
    );

    expect(source).not.toContain("UserLocation");
  });

  it("recenters only on an explicit request, never on GPS drift", () => {
    const source = readFileSync(
      path.join(__dirname, "../../src/components/map/WeatherMap.tsx"),
      "utf8",
    );

    expect(source).toContain("center: centerCoord");
    expect(source).toContain("[recenterNonce]");
    // A zoom here would snap the user's chosen zoom back on every recenter.
    expect(source).not.toMatch(/setStop\(\{\s*center: centerCoord,\s*zoom/);
  });

  it("keeps zoom controls accessible with minimum native targets", () => {
    const source = readFileSync(
      path.join(__dirname, "../../src/components/map/WeatherMap.tsx"),
      "utf8",
    );

    expect(source).toContain('accessibilityRole="button"');
    expect(source).toContain("minWidth: 44");
    expect(source).toContain("minHeight: 44");
  });
});
