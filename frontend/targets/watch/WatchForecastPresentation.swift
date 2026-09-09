import Foundation

/// The API returns local wall-clock strings. Compare against the API's current
/// time, not the Watch's timezone, and never treat midnight as the next hour.
enum WatchForecastPresentation {
    static func nextHour(_ minutely: Forecast.Minutely?, currentTime: String) -> [Double] {
        guard let minutely else { return [] }
        return zip(minutely.time, minutely.precipitation)
            .filter { $0.0 >= currentTime }.prefix(4).map { $0.1 }
    }

    private static let dayParser: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter
    }()
    private static let weekday: DateFormatter = {
        let formatter = DateFormatter()
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "E"
        return formatter
    }()

    static func shortHour(_ iso: String) -> String {
        guard let time = iso.split(separator: "T").last,
              let hour = Int(time.prefix(2)), (0...23).contains(hour) else { return "—" }
        return "\(hour % 12 == 0 ? 12 : hour % 12)\(hour < 12 ? "a" : "p")"
    }

    static func dayOfWeek(_ iso: String) -> String {
        guard let date = dayParser.date(from: iso) else { return "—" }
        return weekday.string(from: date)
    }
}
