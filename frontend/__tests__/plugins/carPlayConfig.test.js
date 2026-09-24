const configure = require("../../app.config");

afterEach(() => { delete process.env.RADAR_CARPLAY; });

test("ordinary iPhone builds retain their provisioning and background behavior", () => {
  const config = { ios: { bundleIdentifier: "com.example.radar" } };
  expect(configure({ config })).toBe(config);
});

test("CarPlay builds request navigation and preserve unrelated entitlements", () => {
  process.env.RADAR_CARPLAY = "1";
  const config = { ios: {
    entitlements: { "aps-environment": "development" },
    infoPlist: { UIBackgroundModes: ["audio", "fetch"] },
  } };
  const output = configure({ config });
  expect(output.ios.entitlements).toEqual({
    "aps-environment": "development", "com.apple.developer.carplay-maps": true,
  });
  expect(output.ios.infoPlist.UIBackgroundModes).toEqual(["audio", "fetch", "location"]);
  expect(config.ios.entitlements).toEqual({ "aps-environment": "development" });
});

jest.mock("@expo/config-plugins", () => ({
  withInfoPlist: (config) => config,
  withDangerousMod: (config) => config,
  withXcodeProject: (config) => config,
  withEntitlementsPlist: (config, mod) => ({ ...config, entitlementMod: mod }),
}));

test("returning from a CarPlay prebuild removes only its restricted entitlement", () => {
  const plugin = require("../../plugins/withCarPlayScene");
  const native = { modResults: { "aps-environment": "development" } };
  process.env.RADAR_CARPLAY = "1";
  plugin({}).entitlementMod(native);
  expect(native.modResults["com.apple.developer.carplay-maps"]).toBe(true);
  delete process.env.RADAR_CARPLAY;
  plugin({}).entitlementMod(native);
  expect(native.modResults).toEqual({ "aps-environment": "development" });
});
