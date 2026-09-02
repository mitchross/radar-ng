import {
  parseMapProjection,
  parseMapStyle,
  parseOpacity,
  parsePalette,
  parseServerUrl,
  parseTimelineMode,
  parseViewMode,
} from "../../src/lib/persistedPrefs";

describe("persisted pref parsers", () => {
  it("accept known values verbatim", () => {
    expect(parseMapStyle("satellite")).toBe("satellite");
    expect(parseMapProjection("globe")).toBe("globe");
    expect(parsePalette("vivid")).toBe("vivid");
    expect(parseTimelineMode("current")).toBe("current");
    expect(parseViewMode("advanced")).toBe("advanced");
  });

  it("fall back on garbage instead of letting it reach a lookup table", () => {
    // Regression: `getString("mapStyle") as MapStyle` let a stale value hit
    // MAP_STYLES_SELFHOSTED[undefined].startsWith → red screen at WeatherMap mount.
    expect(parseMapStyle("midnight")).toBe("light");
    expect(parseMapStyle("")).toBe("light");
    expect(parseMapProjection("mercator")).toBe("flat");
    expect(parsePalette("nexrad")).toBe("classic");
    expect(parseTimelineMode("past")).toBe("forecast");
    expect(parseViewMode("expert")).toBe("simple");
  });

  it("only accepts absolute http(s) server URLs and strips trailing slashes", () => {
    const fb = "https://radar-ng-api.vanillax.me";
    expect(parseServerUrl("https://radar.example", fb)).toBe("https://radar.example");
    expect(parseServerUrl("http://192.168.1.10:8080/", fb)).toBe("http://192.168.1.10:8080");
    expect(parseServerUrl("https://host/prefix/", fb)).toBe("https://host/prefix");
    expect(parseServerUrl("radar.example", fb)).toBe(fb);
    expect(parseServerUrl("ftp://radar.example", fb)).toBe(fb);
    expect(parseServerUrl("", fb)).toBe(fb);
    expect(parseServerUrl("https://", fb)).toBe(fb);
  });

  it("clamps opacity to (0, 1]", () => {
    expect(parseOpacity("0.65", 0.8)).toBe(0.65);
    expect(parseOpacity("1", 0.8)).toBe(1);
    expect(parseOpacity("0", 0.8)).toBe(0.8);
    expect(parseOpacity("1.5", 0.8)).toBe(0.8);
    expect(parseOpacity("abc", 0.8)).toBe(0.8);
  });
});
