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

  it("offers Light, Dark, and System independently of radar map style", () => {
    const settings = source("app/(tabs)/settings.tsx");
    expect(settings).toContain('value: "light"');
    expect(settings).toContain('value: "dark"');
    expect(settings).toContain('value: "system"');
    expect(settings).toContain("setAppearanceMode");
    expect(settings).toContain("setMapStyle");
    expect(settings).toContain('flexWrap: "wrap"');
  });

  // Tabs, radar buttons, the timeline and the style picker are covered by
  // render tests in __tests__/render; only the radar screen itself is not.
  it("keeps the radar close control labeled", () => {
    expect(source("app/(tabs)/radar.tsx")).toContain('accessibilityLabel="Close radar"');
  });

  it("wires native foreground and connectivity state into React Query", () => {
    const rootLayout = source("app/_layout.tsx");
    expect(rootLayout).toContain("bindAppFocus(AppState, setFocused)");
    expect(rootLayout).toContain("bindNetworkOnline(NetInfo, setOnline)");
  });

  it.each([
    "app/(tabs)/index.tsx",
    "screens/NowcastScreen.tsx",
    "app/(tabs)/settings.tsx",
    "components/map/RadarFABs.tsx",
    "app/(tabs)/alerts.tsx",
  ])("guards awaited manual refreshes while offline in %s", (file) => {
    expect(source(file)).toContain("runOnlineRefresh");
  });

  it.each([
    ["app/(tabs)/index.tsx", "Current weather"],
    ["screens/NowcastScreen.tsx", "Next hour precipitation"],
    ["app/(tabs)/alerts.tsx", "Weather alerts"],
    ["app/(tabs)/settings.tsx", "Weather settings"],
  ])("labels the primary %s screen region", (file, label) => {
    expect(source(file)).toContain(`accessibilityLabel="${label}"`);
  });

  it("announces degraded NWS alert state wherever cached alerts remain visible", () => {
    for (const file of [
      "app/(tabs)/index.tsx",
      "app/(tabs)/alerts.tsx",
      "app/(tabs)/radar.tsx",
      "app/alert/[id].tsx",
    ]) {
      const contents = source(file);
      expect(contents).toContain("alertStatus.accessibilityLabel");
      expect(contents).toContain('accessibilityRole="alert"');
    }
  });
});
