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
  // Required because once UIApplicationSceneManifest exists, iOS uses
  // scene-based lifecycle for the iPhone window too. Without a UIWindowScene
  // delegate that creates UIWindow(windowScene:), the RN root view never
  // attaches to the active scene and the app shows a black screen.
  "MainSceneDelegate.swift",
];

// NOTE: `com.apple.developer.carplay-maps` is intentionally NOT added here.
// Apple does not grant it to individual developer accounts, and Xcode auto-
// signing aborts if the entitlement is in the .entitlements file but missing
// from the dev portal. `scripts/carplay-resign.sh` injects it post-archive
// at codesign time for sideload builds intended for the actual car.

// Declare BOTH the iPhone window scene and the CarPlay template scene. On
// iOS 13+, presence of UIApplicationSceneManifest puts the app into
// scene-based lifecycle — the main window must be constructed from a
// UIWindowScene via MainSceneDelegate, otherwise iPhone launch is black.
function withCarPlaySceneManifest(config) {
  return withInfoPlist(config, (c) => {
    const projectName = c.modRequest.projectName || "radarng";
    // UIRequiresFullScreen is deprecated in iOS 26 and ignored at runtime —
    // the Expo template still emits it, so strip it whenever we touch Info.plist.
    delete c.modResults.UIRequiresFullScreen;
    c.modResults.UIApplicationSceneManifest = {
      UIApplicationSupportsMultipleScenes: false,
      UISceneConfigurations: {
        UIWindowSceneSessionRoleApplication: [
          {
            UISceneConfigurationName: "Main",
            UISceneDelegateClassName: `${projectName}.MainSceneDelegate`,
          },
        ],
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
