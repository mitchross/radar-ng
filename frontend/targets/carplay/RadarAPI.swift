import Foundation

enum RadarAPI {
    static let serverURL = "https://radar-ng-api.vanillax.me"

    static func forecastURL(lat: Double, lon: Double) -> URL {
        URL(string: "\(serverURL)/api/forecast/\(lat)/\(lon)")!
    }

    static func alertsURL(lat: Double, lon: Double) -> URL {
        URL(string: "https://api.weather.gov/alerts/active?point=\(lat),\(lon)")!
    }
}
