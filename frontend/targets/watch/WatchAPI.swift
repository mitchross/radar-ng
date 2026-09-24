import Foundation

enum WatchAPI {
    static let serverURL = "https://radar-ng-api.vanillax.me"
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
        let point = "\(round(lat, decimals: pointDecimals)),\(round(lon, decimals: pointDecimals))"
        var req = URLRequest(url: URL(string: "https://api.weather.gov/alerts/active?point=\(point)")!)
        req.setValue(userAgent, forHTTPHeaderField: "User-Agent")
        let data = try await fetch(req)
        let envelope = try JSONDecoder().decode(AlertsEnvelope.self, from: data)
        return envelope.features.map { $0.properties }
    }

    static func fetchLatestRadarFrame() async throws -> WatchRadarFrame {
        let url = URL(string: "\(serverURL)/api/manifest.json")!
        let data = try await fetch(url)
        let manifest = try JSONDecoder().decode(RadarManifest.self, from: data)

        guard let layer = manifest.layers["radar"] else {
            throw WatchAPIError.radarUnavailable
        }
        let frame = layer.frames?.filter {
            WatchRadarFrame.date($0.timestamp).map { $0.timeIntervalSinceNow <= 60 } == true
        }.max { WatchRadarFrame.date($0.timestamp)! < WatchRadarFrame.date($1.timestamp)! }
        guard let frame else {
            throw WatchAPIError.radarUnavailable
        }

        let palettes = frame.palettes ?? layer.palettes ?? []
        let palette = palettes.contains("classic") ? "classic" : (palettes.first ?? "classic")
        return WatchRadarFrame(
            timestamp: frame.timestamp,
            path: frame.path,
            palette: palette,
            maxZoom: min(max(frame.maxZoom ?? 7, 1), 7)
        )
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

struct WatchRadarFrame: Equatable {
    let timestamp: String
    let path: String
    let palette: String
    let maxZoom: Int

    static func date(_ value: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.date(from: value) ?? ISO8601DateFormatter().date(from: value)
    }

    func isFresh(at now: Date) -> Bool {
        guard let date = Self.date(timestamp) else { return false }
        return (-60...900).contains(now.timeIntervalSince(date))
    }

    func tileURL(z: Int, x: Int, y: Int) -> URL? {
        guard maxZoom >= 1, (1...min(maxZoom, 7)).contains(z), (0..<(1 << z)).contains(x), (0..<(1 << z)).contains(y),
              !path.isEmpty, !palette.isEmpty,
              path.allSatisfy({ $0.isLetter || $0.isNumber || "_:+-".contains($0) || $0 == "." }),
              path != ".", path != "..",
              palette.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" }) else { return nil }
        return URL(string: "\(WatchAPI.serverURL)/tiles/radar/\(palette)/\(path)/\(z)/\(x)/\(y).png")
    }
}

private struct RadarManifest: Decodable {
    let layers: [String: RadarLayer]
}

private struct RadarLayer: Decodable {
    let frames: [RadarManifestFrame]?
    let latest: String?
    let palettes: [String]?
}

private struct RadarManifestFrame: Decodable {
    let timestamp: String
    let path: String
    let maxZoom: Int?
    let palettes: [String]?

    enum CodingKeys: String, CodingKey {
        case timestamp, path, palettes
        case maxZoom = "max_zoom"
    }
}

struct Forecast: Decodable {
    let latitude: Double
    let longitude: Double
    let current: Current
    let hourly: Hourly
    let daily: Daily
    let minutely_15: Minutely?

    struct Current: Decodable {
        let time: String
        let temperature_2m: Double
        let apparent_temperature: Double?
        let weather_code: Int?
        let wind_speed_10m: Double?
        let relative_humidity_2m: Double
    }
    struct Hourly: Decodable {
        let time: [String]
        let temperature_2m: [Double]
        let weather_code: [Int?]
        let precipitation_probability: [Int?]
    }
    struct Daily: Decodable {
        let time: [String]
        let temperature_2m_max: [Double]
        let temperature_2m_min: [Double]
        let weather_code: [Int?]
    }
    struct Minutely: Decodable {
        let time: [String]
        let precipitation: [Double]
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
    func isActive(at now: Date) -> Bool {
        WatchRadarFrame.date(expires).map { $0 > now } ?? false
    }
}
