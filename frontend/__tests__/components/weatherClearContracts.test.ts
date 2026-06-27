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
  });
});
