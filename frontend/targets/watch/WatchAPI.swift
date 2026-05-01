import Foundation

enum WatchAPI {
    static let serverURL = "https://radar-ng-api.vanillax.me"

    static func fetchForecast(lat: Double, lon: Double) async throws -> Forecast {
        let url = URL(string: "\(serverURL)/api/forecast/\(lat)/\(lon)")!
        let (data, _) = try await URLSession.shared.data(from: url)
        return try JSONDecoder().decode(Forecast.self, from: data)
    }

    static func fetchAlerts(lat: Double, lon: Double) async throws -> [Alert] {
        var req = URLRequest(url: URL(string: "https://api.weather.gov/alerts/active?point=\(lat),\(lon)")!)
        req.setValue("radar-ng/1.0 (watch)", forHTTPHeaderField: "User-Agent")
        let (data, _) = try await URLSession.shared.data(for: req)
        let envelope = try JSONDecoder().decode(AlertsEnvelope.self, from: data)
        return envelope.features.map { $0.properties }
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
        let apparent_temperature: Double
        let weather_code: Int
        let wind_speed_10m: Double
        let relative_humidity_2m: Double
    }
    struct Hourly: Decodable {
        let time: [String]
        let temperature_2m: [Double]
        let weather_code: [Int]
        let precipitation_probability: [Int]
    }
    struct Daily: Decodable {
        let time: [String]
        let temperature_2m_max: [Double]
        let temperature_2m_min: [Double]
        let weather_code: [Int]
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
}
