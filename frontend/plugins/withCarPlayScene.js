const {
  withInfoPlist,
  withDangerousMod,
  withXcodeProject,
  withEntitlementsPlist,
} = require("@expo/config-plugins");
const fs = require("fs");
const path = require("path");

const CARPLAY_SRC = "targets/carplay";
const CARPLAY_FILES = [
  "RadarCarPlaySceneDelegate.swift",
  "RadarCarPlayDashboardSceneDelegate.swift",
  "RadarMapController.swift",
  "RadarDrivingSession.swift",
  "RadarManeuverFactory.swift",
  "RadarRouteProgress.swift",
  "RadarTileOverlay.swift",
  "RadarLocationManager.swift",
  "RadarAPI.swift",
];

// app.config.js enables the navigation entitlement with RADAR_CARPLAY=1.
// The signing profile must contain Apple's matching approved capability.
const carPlayEnabled = () => process.env.RADAR_CARPLAY === "1";

// The iPhone window scene is declared by expo-build-properties'
// `ios.enableSceneSupport`, which points UIWindowSceneSessionRoleApplication at
// Expo's own EXExpoAppSceneDelegate. That delegate rebuilds the launch options
// from the scene's connectionOptions, so a link that cold-starts the app still
// reaches Linking.getInitialURL(). This plugin therefore never writes the window
// scene, and never replaces AppDelegate.swift.
//
// Order matters: config-plugin mods run last-registered-first, so this plugin
// must be listed BEFORE expo-build-properties in app.json for the window scene
// to exist by the time the CarPlay roles are appended. enableSceneSupport also
// throws when it finds a scene manifest it does not own.
//
// It only appends the CarPlay roles, and only for RADAR_CARPLAY=1 builds.
// Compiling CarPlay scenes into an ordinary release would ship background modes
// and a navigation capability that App Review cannot see (guideline 2.5.4).
function withCarPlaySceneRoles(config) {
  return withInfoPlist(config, (c) => {
    const projectName = c.modRequest.projectName || "radarng";
    // UIRequiresFullScreen is deprecated in iOS 26 and ignored at runtime —
    // the Expo template still emits it, so strip it whenever we touch Info.plist.
    delete c.modResults.UIRequiresFullScreen;

    if (!carPlayEnabled()) {
      return c;
    }

    const manifest = c.modResults.UIApplicationSceneManifest ?? {};
    const configurations = manifest.UISceneConfigurations ?? {};
    // CarPlay connects its scenes alongside the phone window, so the app runs
    // more than one scene at a time.
    manifest.UIApplicationSupportsMultipleScenes = true;
    manifest.CPSupportsDashboardNavigationScene = true;
    configurations.CPTemplateApplicationSceneSessionRoleApplication = [
      {
        UISceneClassName: "CPTemplateApplicationScene",
        UISceneConfigurationName: "CarPlay",
        UISceneDelegateClassName: `${projectName}.RadarCarPlaySceneDelegate`,
      },
    ];
    configurations.CPTemplateApplicationDashboardSceneSessionRoleApplication = [
      {
        UISceneClassName: "CPTemplateApplicationDashboardScene",
        UISceneConfigurationName: "CarPlay Dashboard",
        UISceneDelegateClassName: `${projectName}.RadarCarPlayDashboardSceneDelegate`,
      },
    ];
    manifest.UISceneConfigurations = configurations;
    c.modResults.UIApplicationSceneManifest = manifest;
    return c;
  });
}

function withCarPlayFiles(config) {
  if (!carPlayEnabled()) {
    return config;
  }
  return withDangerousMod(config, [
    "ios",
    async (c) => {
      const projectRoot = c.modRequest.projectRoot;
      const iosRoot = c.modRequest.platformProjectRoot;
      const iosAppName = c.modRequest.projectName;
      const destDir = path.join(iosRoot, iosAppName, "CarPlay");
      fs.mkdirSync(destDir, { recursive: true });
      for (const f of CARPLAY_FILES) {
        const src = path.join(projectRoot, CARPLAY_SRC, f);
        const dst = path.join(destDir, f);
        if (fs.existsSync(src)) fs.copyFileSync(src, dst);
      }
      return c;
    },
  ]);
}

function withCarPlayPbxproj(config) {
  if (!carPlayEnabled()) {
    return config;
  }
  return withXcodeProject(config, (c) => {
    const proj = c.modResults;
    const appName = c.modRequest.projectName;
    let groupKey = proj.findPBXGroupKey({ name: "CarPlay" });
    if (!groupKey) {
      groupKey = proj.pbxCreateGroup("CarPlay", `${appName}/CarPlay`);
      const mainGroup = proj.getFirstProject().firstProject.mainGroup;
      proj.addToPbxGroup(groupKey, mainGroup);
    }
    const target = proj.getFirstTarget().uuid;
    for (const f of CARPLAY_FILES) {
      proj.addSourceFile(f, { target }, groupKey);
    }
    return c;
  });
}

module.exports = (config) => {
  config = withEntitlementsPlist(config, (c) => {
    // Prebuild merges existing native entitlements. Remove a previous opt-in
    // when returning to ordinary development signing.
    if (carPlayEnabled()) {
      c.modResults["com.apple.developer.carplay-maps"] = true;
    } else {
      delete c.modResults["com.apple.developer.carplay-maps"];
    }
    return c;
  });
  config = withCarPlaySceneRoles(config);
  config = withCarPlayFiles(config);
  config = withCarPlayPbxproj(config);
  return config;
};
