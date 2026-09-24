import Foundation

/// What the phone app shares with its extensions (src/lib/sharedState.ts).
/// The widget and CarPlay read the App Group; the Watch reads what
/// WatchConnectivity delivered into its own defaults. This file is kept
/// identical in targets/radar-widget, targets/carplay and targets/watch.
struct RadarShared: Decodable {
    struct Location: Decodable {
        /// "city": a place the user chose; show it instead of local GPS.
        /// "device": the phone's last fix; a fallback only.
        let mode: String
        let lat: Double
        let lon: Double
        let label: String?
    }

    let serverUrl: String
    let palette: String
    let location: Location?

    static let appGroup = "group.com.vanillax.radar-ng"
    static let key = "radarng.shared"
    static let defaultServer = "https://radar-ng-api.vanillax.me"

    static var defaults: UserDefaults? {
        #if os(watchOS)
        return .standard
        #else
        return UserDefaults(suiteName: appGroup)
        #endif
    }

    static func current() -> RadarShared? {
        guard let json = defaults?.string(forKey: key), let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(RadarShared.self, from: data)
    }

    /// The configured server as an http(s) origin, or the default.
    static var serverURL: String {
        guard let raw = current()?.serverUrl,
              let url = URL(string: raw), ["http", "https"].contains(url.scheme ?? ""), url.host != nil
        else { return defaultServer }
        return raw.hasSuffix("/") ? String(raw.dropLast()) : raw
    }

    /// The user's palette when the frame offers it, else "classic", else the first offered.
    static func palette(among available: [String]) -> String {
        if let chosen = current()?.palette, available.contains(chosen) { return chosen }
        return available.contains("classic") ? "classic" : (available.first ?? "classic")
    }

    var chosenCity: Location? { location?.mode == "city" ? location : nil }
}
