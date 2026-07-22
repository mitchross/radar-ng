import { isExternalMapStyle, resolveMapStyleUrl } from "../../src/lib/constants";

const SERVER = "https://radar-ng-api.vanillax.me";

describe("resolveMapStyleUrl / isExternalMapStyle", () => {
  describe("bundled (default — nothing configured)", () => {
    it("joins the relative bundled style path to the serverUrl", () => {
      // The empty override map is what an un-opted-in fork/user has: no
      // EXPO_PUBLIC_BASEMAP_*_STYLE_URL set → bundled Protomaps styles.
      expect(resolveMapStyleUrl(SERVER, "light", {})).toBe(
        `${SERVER}/basemap/styles/positron.json`,
      );
      expect(resolveMapStyleUrl(SERVER, "dark", {})).toBe(
        `${SERVER}/basemap/styles/dark-matter.json`,
      );
      expect(resolveMapStyleUrl(SERVER, "satellite", {})).toBe(
        `${SERVER}/basemap/styles/satellite.json`,
      );
      expect(isExternalMapStyle("light", {})).toBe(false);
    });
  });

  describe("external override (opt-in)", () => {
    const external = {
      light: "https://maps.vanillax.me/styles/light.json",
      dark: "https://maps.vanillax.me/styles/dark.json",
    };

    it("returns the absolute external URL verbatim, ignoring serverUrl", () => {
      expect(resolveMapStyleUrl(SERVER, "light", external)).toBe(
        "https://maps.vanillax.me/styles/light.json",
      );
      expect(isExternalMapStyle("dark", external)).toBe(true);
    });

    it("falls back to bundled for styles not overridden (satellite stays bundled)", () => {
      expect(isExternalMapStyle("satellite", external)).toBe(false);
      expect(resolveMapStyleUrl(SERVER, "satellite", external)).toBe(
        `${SERVER}/basemap/styles/satellite.json`,
      );
    });
  });

  describe("guards against misconfiguration", () => {
    it("treats an empty or relative override value as bundled, not external", () => {
      expect(isExternalMapStyle("light", { light: "" })).toBe(false);
      expect(isExternalMapStyle("light", { light: "/some/relative/path.json" })).toBe(false);
      expect(resolveMapStyleUrl(SERVER, "light", { light: "" })).toBe(
        `${SERVER}/basemap/styles/positron.json`,
      );
    });
  });
});
