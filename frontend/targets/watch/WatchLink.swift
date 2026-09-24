import Foundation
import WatchConnectivity

/// Receives the iPhone app's shared state (server, palette, chosen city,
/// last fix) as WatchConnectivity application context and keeps the latest
/// copy in this Watch's defaults, where RadarShared reads it.
final class WatchLink: NSObject, WCSessionDelegate {
    static let shared = WatchLink()
    /// Called on the main actor when the shared state changes.
    var onChange: (@MainActor () -> Void)?

    func activate() {
        guard WCSession.isSupported() else { return }
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    func session(_ session: WCSession, activationDidCompleteWith state: WCSessionActivationState, error: Error?) {
        // Context delivered while the Watch app wasn't running.
        store(session.receivedApplicationContext)
    }

    func session(_ session: WCSession, didReceiveApplicationContext context: [String: Any]) {
        store(context)
    }

    private func store(_ context: [String: Any]) {
        guard let json = context["shared"] as? String,
              json != RadarShared.defaults?.string(forKey: RadarShared.key) else { return }
        RadarShared.defaults?.set(json, forKey: RadarShared.key)
        let onChange = onChange
        Task { @MainActor in onChange?() }
    }
}
