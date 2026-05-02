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

// The Expo template's AppDelegate creates the iPhone window inline in
// didFinishLaunchingWithOptions and bootstraps RN there. That breaks once
// UIApplicationSceneManifest is declared (which we need for CarPlay): iOS
// switches to scene-based lifecycle and the manually-created UIWindow never
// attaches to the active scene → black screen. Replace AppDelegate.swift
// post-prebuild with a scene-aware version that defers RN bootstrap to
// MainSceneDelegate.startReactNative(in:).
function withSceneAwareAppDelegate(config) {
  return withDangerousMod(config, [
    "ios",
    async (c) => {
      const iosRoot = c.modRequest.platformProjectRoot;
      const appName = c.modRequest.projectName;
      const dst = path.join(iosRoot, appName, "AppDelegate.swift");
      const contents = `internal import Expo
import React
import ReactAppDependencyProvider

@main
class AppDelegate: ExpoAppDelegate {
  var window: UIWindow?

  var reactNativeDelegate: ExpoReactNativeFactoryDelegate?
  var reactNativeFactory: RCTReactNativeFactory?

  private var pendingLaunchOptions: [UIApplication.LaunchOptionsKey: Any]?

  public override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
  ) -> Bool {
    let delegate = ReactNativeDelegate()
    let factory = ExpoReactNativeFactory(delegate: delegate)
    delegate.dependencyProvider = RCTAppDependencyProvider()

    reactNativeDelegate = delegate
    reactNativeFactory = factory
    pendingLaunchOptions = launchOptions

    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  // Called by MainSceneDelegate.scene(_:willConnectTo:options:) once iOS has
  // produced a UIWindowScene. The window MUST be constructed from that scene
  // (UIWindow(windowScene:)) so the RN root view becomes visible — see the
  // comment in MainSceneDelegate.swift for why.
  func startReactNative(in window: UIWindow) {
    self.window = window
    reactNativeFactory?.startReactNative(
      withModuleName: "main",
      in: window,
      launchOptions: pendingLaunchOptions)
    pendingLaunchOptions = nil
  }

  // Deep links (radarng://) and universal links are routed through the
  // scene delegate (see MainSceneDelegate.swift) — iOS 26 deprecated the
  // application(_:open:options:) and application(_:continue:restorationHandler:)
  // hooks in favor of UIScene callbacks once a UIApplicationSceneManifest is
  // declared, which we do for CarPlay + the main window scene.
}

class ReactNativeDelegate: ExpoReactNativeFactoryDelegate {
  // Extension point for config-plugins

  override func sourceURL(for bridge: RCTBridge) -> URL? {
    // needed to return the correct URL for expo-dev-client.
    bridge.bundleURL ?? bundleURL()
  }

  override func bundleURL() -> URL? {
#if DEBUG
    return RCTBundleURLProvider.sharedSettings().jsBundleURL(forBundleRoot: ".expo/.virtual-metro-entry")
#else
    return Bundle.main.url(forResource: "main", withExtension: "jsbundle")
#endif
  }
}
`;
      fs.writeFileSync(dst, contents);
      return c;
    },
  ]);
}

module.exports = (config) => {
  config = withCarPlaySceneManifest(config);
  config = withCarPlayFiles(config);
  config = withCarPlayPbxproj(config);
  config = withSceneAwareAppDelegate(config);
  return config;
};
