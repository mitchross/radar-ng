import UIKit

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
  }
}
