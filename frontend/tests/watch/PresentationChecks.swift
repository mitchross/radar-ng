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
        func watchFrame(path: String, maxZoom: Int?) -> WatchRadarFrame {
            WatchRadarFrame(frame: RadarFrame(timestamp: "2026-09-08T23:30:00Z", path: path, palettes: nil, maxZoom: maxZoom),
                            palette: "classic")
        }
        let frame = watchFrame(path: "2026-09-08T23:30:00+00:00", maxZoom: 7)
        precondition(frame.tileURL(z: 7, x: 33, y: 47)?.absoluteString == "https://radar-ng-api.vanillax.me/tiles/radar/classic/2026-09-08T23:30:00+00:00/7/33/47.png")
        let observed = RadarFrame.date(frame.timestamp)!
        precondition(frame.isFresh(at: observed.addingTimeInterval(899)))
        precondition(!frame.isFresh(at: observed.addingTimeInterval(901)))
        precondition(!frame.isFresh(at: observed.addingTimeInterval(-61)))
        precondition(RadarFrame.date("2026-09-08T23:30:00.000+00:00") == observed)
        precondition(RadarFrame.date("not-a-time") == nil)
        precondition(frame.tileURL(z: 8, x: 33, y: 47) == nil)
        precondition(frame.tileURL(z: 7, x: -1, y: 47) == nil)
        precondition(frame.tileURL(z: 7, x: 33, y: 128) == nil)
        precondition(watchFrame(path: "../other", maxZoom: 7).tileURL(z: 7, x: 33, y: 47) == nil)
        precondition(watchFrame(path: "a/b", maxZoom: 7).tileURL(z: 7, x: 33, y: 47) == nil)
        let lowZoom = watchFrame(path: frame.path, maxZoom: 3)
        precondition(lowZoom.tileURL(z: 3, x: 0, y: 0) != nil)
        precondition(lowZoom.tileURL(z: 4, x: 0, y: 0) == nil)
        // The newest frame wins by parsed time, not by string order: 19:00-05:00 is 00:00Z the next day.
        let manifest = Data("""
        {"layers": {"radar": {"palettes": ["vivid"], "frames": [
          {"timestamp": "2026-09-08T23:30:00+00:00", "path": "2026-09-08T23:30:00+00:00"},
          {"timestamp": "2026-09-08T19:00:00-05:00", "path": "2026-09-08T19:00:00-05:00", "max_zoom": 5},
          {"timestamp": "2026-09-09T12:00:00+00:00", "path": "../x"}]}}}
        """.utf8)
        let latest = try! RadarFrame.latest(inManifest: manifest)!
        precondition(latest.path == "2026-09-08T19:00:00-05:00" && latest.maxZoom == 5 && latest.palettes == ["vivid"])
        let alert = Alert(id: "1", event: "Test", headline: nil, severity: "Severe", areaDesc: "Test",
                          expires: frame.timestamp, effective: "2026-09-08T22:30:00Z", onset: nil, ends: nil)
        precondition(alert.isActive(at: observed.addingTimeInterval(-1)))
        precondition(!alert.isActive(at: observed))
        // Not yet effective, and ended before it expires, are both inactive.
        precondition(!alert.isActive(at: observed.addingTimeInterval(-3601 - 1)))
        let ended = Alert(id: "2", event: "Test", headline: nil, severity: "Severe", areaDesc: "Test",
                          expires: frame.timestamp, effective: "2026-09-08T22:30:00Z", onset: nil, ends: "2026-09-08T23:00:00Z")
        precondition(!ended.isActive(at: observed.addingTimeInterval(-60)))
        // One null in the forecast must not fail the whole decode.
        let json = #"{"latitude":1,"longitude":2,"current":{"time":"2026-09-08T19:30","temperature_2m":null,"relative_humidity_2m":null},"hourly":{"time":["2026-09-08T19:00"],"temperature_2m":[null],"weather_code":[null],"precipitation_probability":[null]},"daily":{"time":["2026-09-08"],"temperature_2m_max":[null],"temperature_2m_min":[50],"weather_code":[1]}}"#
        let decoded = try! JSONDecoder().decode(Forecast.self, from: Data(json.utf8))
        precondition(WatchForecastPresentation.degrees(decoded.current.temperature_2m) == "—")
        precondition(WatchForecastPresentation.degrees(decoded.daily.temperature_2m_min[0]) == "50°")
        // The phone's unit applies: 50°F is 10°C, -2.5°C rounds up like Math.round, missing stays "—".
        precondition(WatchForecastPresentation.degrees(50, unit: "celsius") == "10°")
        precondition(WatchForecastPresentation.degrees(27.5, unit: "celsius") == "-2°")
        precondition(WatchForecastPresentation.degrees(50, unit: "fahrenheit") == "50°")
        precondition(WatchForecastPresentation.degrees(nil, unit: "celsius") == "—")
        let gappy = Forecast.Minutely(time: ["2026-09-08T19:30", "2026-09-08T19:45", "2026-09-08T20:00", "2026-09-08T20:15"],
                                      precipitation: [0, nil, 0, 0])
        precondition(WatchForecastPresentation.nextHour(gappy, currentTime: "2026-09-08T19:30").isEmpty)
        // Basemap comes from the home cluster at the 512-px static zoom matching 256-pt radar tiles.
        precondition(WatchBasemap.url(latitude: 42.9634, longitude: -85.6681, zoom: 7, size: CGSize(width: 184, height: 224))?.absoluteString
            == "https://maps.vanillax.me/raster/styles/dark/static/-85.66810,42.96340,6/184x224@2x.png")
        precondition(WatchBasemap.url(latitude: 0, longitude: 0, zoom: 5, size: CGSize(width: 4000, height: 0.2))?.absoluteString
            == "https://maps.vanillax.me/raster/styles/dark/static/0.00000,0.00000,4/1024x1@2x.png")
        print("Watch presentation checks passed")
    }
}
