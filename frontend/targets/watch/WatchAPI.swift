import Foundation

enum WatchAPI {
    static var serverURL: String { RadarShared.serverURL }
    private static let userAgent = "radar-ng/2.0 (watchOS)"

    /// Mirrors `PRECISION` in `src/lib/coordinates.ts` so the watch and the phone
    /// ask the same question for the same place. The radar and nowcast grids are
    /// 1 km cells, so a finer fix only costs cache hits, never accuracy.
    private static let weatherDecimals = 2
    /// Alert polygons can be far smaller than a forecast cell, so they keep 3.
    private static let pointDecimals = 3

    private static func round(_ value: Double, decimals: Int) -> Double {
        let factor = pow(10.0, Double(decimals))
        return (value * factor).rounded() / factor
    }

    static func fetchForecast(lat: Double, lon: Double) async throws -> Forecast {
        let url = URL(
            string: "\(serverURL)/api/forecast/\(round(lat, decimals: weatherDecimals))/\(round(lon, decimals: weatherDecimals))"
        )!
        let data = try await fetch(url)
        return try JSONDecoder().decode(Forecast.self, from: data)
    }

    static func fetchAlerts(lat: Double, lon: Double) async throws -> [Alert] {
        // The server proxies NWS point matching, so the Watch never calls a third party.
        let url = URL(
            string: "\(serverURL)/api/alerts?lat=\(round(lat, decimals: pointDecimals))&lon=\(round(lon, decimals: pointDecimals))"
        )!
        let data = try await fetch(url)
        let envelope = try JSONDecoder().decode(AlertsEnvelope.self, from: data)
        return envelope.features.map { $0.properties }
    }

    static func fetchLatestRadarFrame() async throws -> WatchRadarFrame {
        let server = serverURL  // once: the frame's tiles must come from the server that listed it
        let url = URL(string: "\(server)/api/manifest.json")!
        let data = try await fetch(url)
        guard let frame = try RadarFrame.latest(inManifest: data) else {
            throw WatchAPIError.radarUnavailable
        }
        return WatchRadarFrame(frame: frame, palette: frame.palette, server: server)
    }

    private static func fetch(_ url: URL) async throws -> Data {
        var request = URLRequest(url: url)
        request.setValue(userAgent, forHTTPHeaderField: "User-Agent")
        return try await fetch(request)
    }

    private static func fetch(_ request: URLRequest) async throws -> Data {
        var request = request
        request.timeoutInterval = 15
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw WatchAPIError.invalidResponse
        }
        return data
    }
}

/// Basemap images rendered by the home cluster's tileserver-gl from the phone's VersaTiles styles.
enum WatchBasemap {
    static let rasterURL = "https://maps.vanillax.me/raster"

    /// Static-map zoom uses MapLibre's 512-px convention, one less than the 256-pt radar tile zoom,
    /// so the image lines up with the radar tiles RadarMapView positions at `zoom`.
    static func url(latitude: Double, longitude: Double, zoom: Int, size: CGSize) -> URL? {
        let width = min(1024, max(1, Int(size.width.rounded())))
        let height = min(1024, max(1, Int(size.height.rounded())))
        return URL(string: String(format: "%@/styles/dark/static/%.5f,%.5f,%d/%dx%d@2x.png",
                                  rasterURL, longitude, latitude, zoom - 1, width, height))
    }
}

enum WatchAPIError: LocalizedError {
    case invalidResponse
    case radarUnavailable

    var errorDescription: String? {
        switch self {
        case .invalidResponse: return "The weather service is unavailable."
        case .radarUnavailable: return "Live radar is warming up."
        }
    }
}

/// A manifest frame with the palette and server it was chosen for, so every tile
/// of the frame comes from the server that listed it, in the palette picked then.
struct WatchRadarFrame: Equatable {
    let frame: RadarFrame
    let palette: String
    var server = WatchAPI.serverURL

    var timestamp: String { frame.timestamp }
    var path: String { frame.path }
    /// The zoom the radar is published to, within what the Watch map draws.
    var maxZoom: Int { min(max(frame.maxZoom ?? 7, 1), 7) }

    func isFresh(at now: Date) -> Bool { frame.isFresh(at: now) }

    func tileURL(z: Int, x: Int, y: Int) -> URL? {
        guard z <= maxZoom else { return nil }
        return frame.tileURL(server: server, palette: palette, z: z, x: x, y: y)
    }
}

struct Forecast: Decodable {
    let latitude: Double
    let longitude: Double
    let current: Current
    let hourly: Hourly
    let daily: Daily
    let minutely_15: Minutely?

    // Open-Meteo sends null for values a model doesn't provide. Every value is
    // optional so one null can't fail the whole forecast; the UI shows "—".
    struct Current: Decodable {
        let time: String
        let temperature_2m: Double?
        let weather_code: Int?
        let wind_speed_10m: Double?
        let relative_humidity_2m: Double?
    }
    struct Hourly: Decodable {
        let time: [String]
        let temperature_2m: [Double?]
        let weather_code: [Int?]
        let precipitation_probability: [Int?]
    }
    struct Daily: Decodable {
        let time: [String]
        let temperature_2m_max: [Double?]
        let temperature_2m_min: [Double?]
        let weather_code: [Int?]
        // Optional so a forecast saved before these were decoded still loads.
        let sunrise: [String?]?
        let sunset: [String?]?
    }
    struct Minutely: Decodable {
        let time: [String]
        let precipitation: [Double?]
    }
}

struct AlertsEnvelope: Decodable {
    let features: [AlertFeature]
}
struct AlertFeature: Decodable {
    let properties: Alert
}
struct Alert: Decodable, Identifiable {
    let id: String
    let event: String
    let headline: String?
    let severity: String
    let areaDesc: String
    let expires: String
    let effective: String?
    let onset: String?
    let ends: String?

    /// Same window as the phone (src/lib/alertLifecycle.ts): from `effective`
    /// (or `onset`) until the earlier of `expires` and `ends`.
    func isActive(at now: Date) -> Bool {
        guard let expiresAt = RadarFrame.date(expires),
              let startsAt = (effective ?? onset).flatMap(RadarFrame.date) else { return false }
        let endsAt = ends.flatMap(RadarFrame.date).map { min($0, expiresAt) } ?? expiresAt
        return now >= startsAt && now < endsAt
    }
}
