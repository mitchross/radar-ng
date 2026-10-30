import Foundation

/// What the phone app shares with its extensions (src/lib/sharedState.ts).
/// The widget and CarPlay read the App Group; the Watch reads what
/// WatchConnectivity delivered into its own defaults. This is the only copy:
/// @bacons/apple-targets links targets/_shared into the app and every target.
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

/// One frame of the radar layer in /api/manifest.json. The widget, CarPlay and
/// the Watch all decode, choose, check and address frames through this type.
struct RadarFrame: Decodable, Equatable {
    let timestamp: String
    let path: String
    let palettes: [String]?
    let maxZoom: Int?

    enum CodingKeys: String, CodingKey {
        case timestamp, path, palettes
        case maxZoom = "max_zoom"
    }

    /// A frame older than this is "Older radar" everywhere.
    static let freshSeconds: TimeInterval = 900
    /// How far ahead of this device's clock a frame may be stamped.
    static let clockSkew: TimeInterval = 60

    // Built once: these run over every manifest frame.
    private static let fractionalISO: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()
    private static let plainISO = ISO8601DateFormatter()

    static func date(_ value: String) -> Date? {
        fractionalISO.date(from: value) ?? plainISO.date(from: value)
    }

    var observedAt: Date? { Self.date(timestamp) }

    /// The user's palette when this frame offers it, else "classic", else the first offered.
    var palette: String { RadarShared.palette(among: palettes ?? []) }

    /// The radar tiler writes `path` as the frame's timestamp: one segment of
    /// digits, letters and `_.:+-`. No slash, so it cannot leave /tiles/radar/<palette>/.
    var hasSafePath: Bool {
        !path.isEmpty && path != "." && path != ".."
            && path.allSatisfy { $0.isLetter || $0.isNumber || "_.:+-".contains($0) }
    }

    /// Has a time, is not stamped in the future, and has a safe path.
    var isValid: Bool {
        guard let date = observedAt, date.timeIntervalSinceNow <= Self.clockSkew else { return false }
        return hasSafePath
    }

    func isFresh(at now: Date = Date()) -> Bool {
        guard let date = observedAt else { return false }
        return (-Self.clockSkew...Self.freshSeconds).contains(now.timeIntervalSince(date))
    }

    /// Nil when the zoom, tile address, palette or path is not safe to put in a URL.
    func tileURL(server: String, palette: String, z: Int, x: Int, y: Int) -> URL? {
        guard (1...7).contains(z), (0..<(1 << z)).contains(x), (0..<(1 << z)).contains(y),
              hasSafePath, !palette.isEmpty,
              palette.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" }) else { return nil }
        return URL(string: "\(server)/tiles/radar/\(palette)/\(path)/\(z)/\(x)/\(y).png")
    }

    private struct Manifest: Decodable { let layers: [String: Layer] }
    private struct Layer: Decodable { let frames: [RadarFrame]?; let palettes: [String]? }

    /// The newest valid frame of the manifest's "radar" layer, by parsed time. It
    /// carries the layer's palettes when it lists none of its own. Nil if there is none.
    static func latest(inManifest data: Data) throws -> RadarFrame? {
        guard let layer = try JSONDecoder().decode(Manifest.self, from: data).layers["radar"],
              let newest = layer.frames?.filter(\.isValid)
                  .max(by: { ($0.observedAt ?? .distantPast) < ($1.observedAt ?? .distantPast) })
        else { return nil }
        return RadarFrame(timestamp: newest.timestamp, path: newest.path,
                          palettes: newest.palettes ?? layer.palettes, maxZoom: newest.maxZoom)
    }
}
