import UIKit
import React

// Required because Info.plist declares a CarPlay scene role under
// UIApplicationSceneManifest. Once a manifest exists, iOS 13+ uses scene-based
// lifecycle for ALL roles, so the iPhone window must be created from the
// connecting UIWindowScene — not from UIWindow(frame:) in AppDelegate, which
// won't attach to any active scene and renders a black screen.
class MainSceneDelegate: UIResponder, UIWindowSceneDelegate {
  var window: UIWindow?

  func scene(
    _ scene: UIScene,
    willConnectTo session: UISceneSession,
    options connectionOptions: UIScene.ConnectionOptions
  ) {
    guard let windowScene = scene as? UIWindowScene else { return }
    let window = UIWindow(windowScene: windowScene)
    self.window = window

    guard let appDelegate = UIApplication.shared.delegate as? AppDelegate else {
      return
    }
    appDelegate.startReactNative(in: window)

    // Forward any URL the app was launched with (radarng://… cold-start path).
    if !connectionOptions.urlContexts.isEmpty {
      self.scene(scene, openURLContexts: connectionOptions.urlContexts)
    }
    // And any continued NSUserActivity (universal links).
    for activity in connectionOptions.userActivities {
      RCTLinkingManager.application(
        UIApplication.shared,
        continue: activity,
        restorationHandler: { _ in }
      )
    }
  }

  // Foreground deep-link handoff. iOS 26 routes radarng:// URLs here once a
  // UIApplicationSceneManifest is declared (which we need for CarPlay), so
  // forward to RCTLinkingManager — RN's deep-link plumbing is unchanged.
  func scene(_ scene: UIScene, openURLContexts URLContexts: Set<UIOpenURLContext>) {
    for context in URLContexts {
      RCTLinkingManager.application(
        UIApplication.shared,
        open: context.url,
        options: [:]
      )
    }
  }

  // Universal links arriving while the scene is connected.
  func scene(_ scene: UIScene, continue userActivity: NSUserActivity) {
    RCTLinkingManager.application(
      UIApplication.shared,
      continue: userActivity,
      restorationHandler: { _ in }
    )
  }
}
