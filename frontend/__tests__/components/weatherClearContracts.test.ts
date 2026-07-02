import { readFileSync } from "fs";
import path from "path";

function source(relativePath: string): string {
  return readFileSync(path.join(__dirname, "../../src", relativePath), "utf8");
}

describe("Weather Clear native UI contracts", () => {
  it("embeds both design font families in the native build", () => {
    const appConfig = readFileSync(path.join(__dirname, "../../app.json"), "utf8");
    expect(appConfig).toContain("Newsreader_400Regular.ttf");
    expect(appConfig).toContain("SplineSans_400Regular.ttf");
    expect(appConfig).toContain("SplineSans_700Bold.ttf");
  });

  it("keeps all tab controls accessible and at least 44 points tall", () => {
    const tabs = source("app/(tabs)/_layout.tsx");
    expect(tabs).toContain('accessibilityRole="tab"');
    expect(tabs).toContain("accessibilityState={{ selected: active }}");
    expect(tabs).toContain("minHeight: 44");
    expect(tabs).toContain("adjustsFontSizeToFit");
    expect(tabs).toContain("numberOfLines={1}");
  });

  it("hides the tab bar only on the full-screen radar route", () => {
    const tabs = source("app/(tabs)/_layout.tsx");
    expect(tabs).toContain('activeRoute === "radar"');
  });

  it("offers Light, Dark, and System independently of radar map style", () => {
    const settings = source("app/(tabs)/settings.tsx");
    expect(settings).toContain('value: "light"');
    expect(settings).toContain('value: "dark"');
    expect(settings).toContain('value: "system"');
    expect(settings).toContain("setAppearanceMode");
    expect(settings).toContain("setMapStyle");
    expect(settings).toContain('flexWrap: "wrap"');
  });

  it("keeps radar close, playback, layers, and map style controls labeled", () => {
    const radar = source("app/(tabs)/radar.tsx");
    const fabs = source("components/map/RadarFABs.tsx");
    const timeline = source("components/timeline/TimelineBar.tsx");
    const mapStyle = source("components/map/MapStylePicker.tsx");
    expect(radar).toContain('accessibilityLabel="Close radar"');
    expect(fabs).toContain("accessibilityLabel");
    expect(timeline).toContain("accessibilityLabel");
    expect(mapStyle).toContain('accessibilityLabel="Close map style picker"');
    expect(mapStyle).toContain('accessibilityRole="radio"');
    expect(mapStyle).toContain("minHeight: 44");
  });

  it.each([
    ["app/(tabs)/index.tsx", "Current weather"],
    ["screens/NowcastScreen.tsx", "Next hour precipitation"],
    ["app/(tabs)/alerts.tsx", "Weather alerts"],
    ["app/(tabs)/settings.tsx", "Weather settings"],
  ])("labels the primary %s screen region", (file, label) => {
    expect(source(file)).toContain(`accessibilityLabel="${label}"`);
  });
});
