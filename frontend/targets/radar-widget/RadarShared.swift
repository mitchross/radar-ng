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
        /// When the phone took the "device" fix, in milliseconds since 1970. Absent from older phones.
        let at: Double?
    }

    let serverUrl: String
    let palette: String
    /// "fahrenheit" or "celsius"; the forecast API always returns Fahrenheit. Absent from older phones.
    let temperatureUnit: String?
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

    /// A phone fix older than this is no longer a fair stand-in for where the user is.
    static let maxPhoneFixAge: TimeInterval = 60 * 60

    /// The phone's last fix, unless it is too old. A payload with no `at` is trusted as before.
    var recentPhoneFix: Location? {
        guard let location else { return nil }
        guard let at = location.at else { return location }
        return Date().timeIntervalSince1970 - at / 1000 > Self.maxPhoneFixAge ? nil : location
    }
}
