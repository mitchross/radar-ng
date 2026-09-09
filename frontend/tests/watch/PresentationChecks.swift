import Foundation

@main
struct PresentationChecks {
    static func main() {
        let minutes = Forecast.Minutely(
            time: ["2026-09-08T00:00", "2026-09-08T19:30", "2026-09-08T19:45", "2026-09-08T20:00", "2026-09-08T20:15", "2026-09-08T20:30"],
            precipitation: [99, 0, 0.01, 0.02, 0.03, 99]
        )
        precondition(WatchForecastPresentation.nextHour(minutes, currentTime: "2026-09-08T19:30") == [0, 0.01, 0.02, 0.03])
        precondition(WatchForecastPresentation.nextHour(nil, currentTime: "2026-09-08T19:30").isEmpty)
        precondition(WatchForecastPresentation.shortHour("2026-09-08T00:00") == "12a")
        precondition(WatchForecastPresentation.shortHour("2026-09-08T19:00") == "7p")
        precondition(WatchForecastPresentation.shortHour("bad") == "—")
        let frame = WatchRadarFrame(timestamp: "2026-09-08T23:30:00Z", path: "2026-09-08T23:30:00+00:00", palette: "classic", maxZoom: 7)
        precondition(frame.tileURL(z: 7, x: 33, y: 47)?.absoluteString == "https://radar-ng-api.vanillax.me/tiles/radar/classic/2026-09-08T23:30:00+00:00/7/33/47.png")
        print("Watch presentation checks passed")
    }
}
