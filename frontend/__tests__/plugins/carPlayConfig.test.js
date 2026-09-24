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
  withInfoPlist: (config, mod) => ({ ...config, infoPlistMod: mod }),
  withDangerousMod: (config, mod) => ({ ...config, dangerousMod: mod }),
  withXcodeProject: (config, mod) => ({ ...config, xcodeMod: mod }),
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

// The window scene belongs to expo-build-properties' ios.enableSceneSupport,
// which points UIWindowSceneSessionRoleApplication at EXExpoAppSceneDelegate.
const windowSceneManifest = () => ({
  UIApplicationSupportsMultipleScenes: false,
  UISceneConfigurations: {
    UIWindowSceneSessionRoleApplication: [
      {
        UISceneConfigurationName: "Default Configuration",
        UISceneDelegateClassName: "EXExpoAppSceneDelegate",
      },
    ],
  },
});

const runInfoPlistMod = (plugin, modResults) => {
  const native = { modRequest: { projectName: "radarng" }, modResults };
  return plugin({}).infoPlistMod(native).modResults;
};

test("ordinary builds keep the Expo window scene and add no CarPlay roles", () => {
  const plugin = require("../../plugins/withCarPlayScene");
  const modResults = { UIRequiresFullScreen: true, UIApplicationSceneManifest: windowSceneManifest() };
  const output = runInfoPlistMod(plugin, modResults);

  expect(output.UIRequiresFullScreen).toBeUndefined();
  expect(output.UIApplicationSceneManifest).toEqual(windowSceneManifest());
});

test("CarPlay builds append the three scene roles without losing the window scene", () => {
  process.env.RADAR_CARPLAY = "1";
  const plugin = require("../../plugins/withCarPlayScene");
  const output = runInfoPlistMod(plugin, { UIApplicationSceneManifest: windowSceneManifest() });
  const manifest = output.UIApplicationSceneManifest;

  expect(manifest.UIApplicationSupportsMultipleScenes).toBe(true);
  expect(manifest.CPSupportsDashboardNavigationScene).toBe(true);
  expect(manifest.UISceneConfigurations.UIWindowSceneSessionRoleApplication).toEqual(
    windowSceneManifest().UISceneConfigurations.UIWindowSceneSessionRoleApplication,
  );
  expect(
    manifest.UISceneConfigurations.CPTemplateApplicationSceneSessionRoleApplication[0]
      .UISceneDelegateClassName,
  ).toBe("radarng.RadarCarPlaySceneDelegate");
  expect(
    manifest.UISceneConfigurations.CPTemplateApplicationDashboardSceneSessionRoleApplication[0]
      .UISceneDelegateClassName,
  ).toBe("radarng.RadarCarPlayDashboardSceneDelegate");
});

test("ordinary builds compile no CarPlay sources and touch no pbx group", () => {
  const plugin = require("../../plugins/withCarPlayScene");
  const config = plugin({});
  expect(config.dangerousMod).toBeUndefined();
  expect(config.xcodeMod).toBeUndefined();
});

test("CarPlay builds copy and compile the CarPlay sources", () => {
  process.env.RADAR_CARPLAY = "1";
  const plugin = require("../../plugins/withCarPlayScene");
  const config = plugin({});
  // withDangerousMod takes [platform, action]; withXcodeProject takes the action.
  expect(config.dangerousMod[0]).toBe("ios");
  expect(typeof config.dangerousMod[1]).toBe("function");
  expect(typeof config.xcodeMod).toBe("function");
});
