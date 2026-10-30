const {
  withInfoPlist,
  withDangerousMod,
  withXcodeProject,
  withEntitlementsPlist,
} = require("@expo/config-plugins");
const fs = require("fs");
const path = require("path");

const CARPLAY_SRC = "targets/carplay";

// Every Swift file in targets/carplay is copied and compiled, so a new file
// needs no list entry and a renamed one cannot be skipped.
const carPlayFiles = (projectRoot) => {
  const files = fs
    .readdirSync(path.join(projectRoot, CARPLAY_SRC))
    .filter((f) => f.endsWith(".swift"))
    .sort();
  if (files.length === 0) throw new Error(`No CarPlay Swift sources in ${CARPLAY_SRC}`);
  return files;
};

// This plugin adds the navigation entitlement with RADAR_CARPLAY=1 (and removes
// it otherwise). The signing profile must contain Apple's matching approved capability.
const carPlayEnabled = () => process.env.RADAR_CARPLAY === "1";

// The iPhone window scene is Expo's: from SDK 58 prebuild generates
// SceneDelegate.swift (a subclass of ExpoAppSceneDelegate) and points
// UIWindowSceneSessionRoleApplication at it. That delegate rebuilds the launch
// options from the scene's connectionOptions, so a link that cold-starts the app
// still reaches Linking.getInitialURL(). This plugin therefore never writes the
// window scene, and never replaces AppDelegate.swift or SceneDelegate.swift.
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
      for (const f of carPlayFiles(projectRoot)) {
        fs.copyFileSync(path.join(projectRoot, CARPLAY_SRC, f), path.join(destDir, f));
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
    for (const f of carPlayFiles(c.modRequest.projectRoot)) {
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
