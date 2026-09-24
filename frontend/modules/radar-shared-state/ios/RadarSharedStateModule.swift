import ExpoModulesCore
import WatchConnectivity
import WidgetKit

/// Publishes the app's shared state (JSON) to the App Group, read by the
/// widget and CarPlay, and to the paired Watch via WatchConnectivity.
public class RadarSharedStateModule: Module {
  static let appGroup = "group.com.vanillax.radar-ng"
  static let key = "radarng.shared"

  public func definition() -> ModuleDefinition {
    Name("RadarSharedState")

    Function("publish") { (json: String) in
      guard let defaults = UserDefaults(suiteName: Self.appGroup) else { return }
      if defaults.string(forKey: Self.key) != json {
        defaults.set(json, forKey: Self.key)
        WidgetCenter.shared.reloadAllTimelines()
      }
      WatchLink.shared.send(json)
    }
  }
}

/// Keeps the latest payload as the session's application context: the Watch
/// gets it on its next launch even when it isn't reachable now.
final class WatchLink: NSObject, WCSessionDelegate {
  static let shared = WatchLink()
  private var pending: String?

  func send(_ json: String) {
    guard WCSession.isSupported() else { return }
    let session = WCSession.default
    if session.delegate == nil { session.delegate = self }
    guard session.activationState == .activated else {
      pending = json
      session.activate()
      return
    }
    guard session.isPaired, session.isWatchAppInstalled else { return }
    try? session.updateApplicationContext(["shared": json])
  }

  func session(_ session: WCSession, activationDidCompleteWith state: WCSessionActivationState, error: Error?) {
    guard state == .activated, let json = pending else { return }
    pending = nil
    send(json)
  }

  func sessionDidBecomeInactive(_ session: WCSession) {}
  func sessionDidDeactivate(_ session: WCSession) { session.activate() }
}
