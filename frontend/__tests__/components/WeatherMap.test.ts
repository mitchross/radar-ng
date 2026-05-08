import { readFileSync } from "fs";
import path from "path";

describe("WeatherMap", () => {
  it("does not mount MapLibre native UserLocation", () => {
    const source = readFileSync(
      path.join(__dirname, "../../src/components/map/WeatherMap.tsx"),
      "utf8",
    );

    expect(source).not.toContain("MapLibreGL.UserLocation");
  });

  it("recenters the MapLibre camera when the store location changes", () => {
    const source = readFileSync(
      path.join(__dirname, "../../src/components/map/WeatherMap.tsx"),
      "utf8",
    );

    expect(source).toContain("cameraRef.current?.setCamera({");
    expect(source).toContain("centerCoordinate: centerCoord");
    expect(source).toContain("[latitude, longitude]");
  });
});
