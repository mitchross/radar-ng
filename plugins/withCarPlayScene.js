const {
  withInfoPlist,
  withDangerousMod,
  withXcodeProject,
} = require("@expo/config-plugins");
const fs = require("fs");
const path = require("path");

const CARPLAY_SRC = "targets/carplay";
const CARPLAY_FILES = [
  "RadarCarPlaySceneDelegate.swift",
  "RadarMapController.swift",
  "RadarTileOverlay.swift",
  "RadarLocationManager.swift",
  "RadarAPI.swift",
];

// NOTE: `com.apple.developer.carplay-maps` is intentionally NOT added here.
// Apple does not grant it to individual developer accounts, and Xcode auto-
// signing aborts if the entitlement is in the .entitlements file but missing
// from the dev portal. `scripts/carplay-resign.sh` injects it post-archive
// at codesign time for sideload builds intended for the actual car.

// Declare ONLY the CarPlay scene and keep UIApplicationSupportsMultipleScenes=false
// so the AppDelegate's `window` continues to own the main UI. Declaring the main
// window scene here (or flipping multi-scene to true) breaks RN startup on launch.
function withCarPlaySceneManifest(config) {
  return withInfoPlist(config, (c) => {
    const projectName = c.modRequest.projectName || "radarng";
    c.modResults.UIApplicationSceneManifest = {
      UIApplicationSupportsMultipleScenes: false,
      UISceneConfigurations: {
        CPTemplateApplicationSceneSessionRoleApplication: [
          {
            UISceneClassName: "CPTemplateApplicationScene",
            UISceneConfigurationName: "CarPlay",
            UISceneDelegateClassName: `${projectName}.RadarCarPlaySceneDelegate`,
          },
        ],
      },
    };
    return c;
  });
}

function withCarPlayFiles(config) {
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
  return withXcodeProject(config, (c) => {
    const proj = c.modResults;
    const appName = c.modRequest.projectName;
    const groupKey = proj.pbxCreateGroup("CarPlay", `${appName}/CarPlay`);
    const mainGroup = proj.getFirstProject().firstProject.mainGroup;
    proj.addToPbxGroup(groupKey, mainGroup);
    const target = proj.getFirstTarget().uuid;
    for (const f of CARPLAY_FILES) {
      proj.addSourceFile(f, { target }, groupKey);
    }
    return c;
  });
}

module.exports = (config) => {
  config = withCarPlaySceneManifest(config);
  config = withCarPlayFiles(config);
  config = withCarPlayPbxproj(config);
  return config;
};
