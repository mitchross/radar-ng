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
        let observed = WatchRadarFrame.date(frame.timestamp)!
        precondition(frame.isFresh(at: observed.addingTimeInterval(899)))
        precondition(!frame.isFresh(at: observed.addingTimeInterval(901)))
        precondition(!frame.isFresh(at: observed.addingTimeInterval(-61)))
        precondition(WatchRadarFrame.date("2026-09-08T23:30:00.000+00:00") == observed)
        precondition(WatchRadarFrame.date("not-a-time") == nil)
        precondition(frame.tileURL(z: 8, x: 33, y: 47) == nil)
        precondition(frame.tileURL(z: 7, x: -1, y: 47) == nil)
        precondition(frame.tileURL(z: 7, x: 33, y: 128) == nil)
        let invalid = WatchRadarFrame(timestamp: frame.timestamp, path: "../other", palette: "classic", maxZoom: 7)
        precondition(invalid.tileURL(z: 7, x: 33, y: 47) == nil)
        let invalidZoom = WatchRadarFrame(timestamp: frame.timestamp, path: frame.path, palette: "classic", maxZoom: 0)
        precondition(invalidZoom.tileURL(z: 1, x: 0, y: 0) == nil)
        let alert = Alert(id: "1", event: "Test", headline: nil, severity: "Severe", areaDesc: "Test", expires: frame.timestamp)
        precondition(alert.isActive(at: observed.addingTimeInterval(-1)))
        precondition(!alert.isActive(at: observed))
        // Basemap comes from the home cluster at the 512-px static zoom matching 256-pt radar tiles.
        precondition(WatchBasemap.url(latitude: 42.9634, longitude: -85.6681, zoom: 7, size: CGSize(width: 184, height: 224))?.absoluteString
            == "https://maps.vanillax.me/raster/styles/dark/static/-85.66810,42.96340,6/184x224@2x.png")
        precondition(WatchBasemap.url(latitude: 0, longitude: 0, zoom: 5, size: CGSize(width: 4000, height: 0.2))?.absoluteString
            == "https://maps.vanillax.me/raster/styles/dark/static/0.00000,0.00000,4/1024x1@2x.png")
        print("Watch presentation checks passed")
    }
}
