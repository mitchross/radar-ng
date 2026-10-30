import CoreLocation
import UIKit

@MainActor
enum RadarSnapshot {
    static func load() async -> RadarEntry {
        let shared = RadarShared.current()
        // A city chosen in the app wins over the widget's own GPS.
        if let city = shared?.chosenCity {
            return await load(at: CLLocationCoordinate2D(latitude: city.lat, longitude: city.lon))
        }
        let locator = WidgetLocation()
        if locator.isAuthorized, let location = await locator.locate() {
            return await load(at: location.coordinate)
        }
        // No fix of our own: the phone app's last fix is better than nothing.
        if let last = shared?.recentPhoneFix {
            return await load(at: CLLocationCoordinate2D(latitude: last.lat, longitude: last.lon))
        }
        return locator.isAuthorized
            ? RadarEntry(date: .now, message: "Location unavailable · Try later", symbol: "location.slash")
            : RadarEntry(date: .now, message: "Open Radar NG to enable location", symbol: "location.slash")
    }

    static func load(at coordinate: CLLocationCoordinate2D) async -> RadarEntry {
        // MRMS coverage: avoid presenting an empty radar map outside the service area.
        guard (20...55).contains(coordinate.latitude),
              (-130 ... -60).contains(coordinate.longitude) else {
            return RadarEntry(date: .now, message: "Radar covers the continental US")
        }
        do {
            // Read once: the manifest and its tiles must come from the same server.
            let server = RadarShared.serverURL
            let manifest = try await fetch(URL(string: "\(server)/api/manifest.json")!)
            guard let frame = try RadarFrame.latest(inManifest: manifest),
                  let observedAt = frame.observedAt else { throw SnapshotError.unavailable }
            let image = try await render(coordinate, frame: frame, palette: frame.palette, server: server)
            return RadarEntry(date: .now, image: image, observedAt: observedAt,
                              message: frame.isFresh() ? "Nearby · Snapshot" : "Older radar")
        } catch {
            // Missing tiles must not masquerade as dry conditions.
            return RadarEntry(date: .now, message: "Radar unavailable · Try later")
        }
    }

    private static func fetch(_ url: URL) async throws -> Data {
        guard let data = try await fetchTile(url) else { throw SnapshotError.unavailable }
        return data
    }

    /// The tiler writes no file for a tile without precipitation, so a 404 for a
    /// frame the manifest advertises means "clear here" (nil). Other statuses
    /// and network errors still fail, so a broken server never reads as dry.
    private static func fetchTile(_ url: URL) async throws -> Data? {
        var request = URLRequest(url: url)
        request.timeoutInterval = 8
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw SnapshotError.unavailable }
        if http.statusCode == 404 { return nil }
        guard http.statusCode == 200 else { throw SnapshotError.unavailable }
        return data
    }

    /// Self-hosted basemap renderer (tileserver-gl over VersaTiles). Same style family as the phone.
    private static let basemap = "https://maps.vanillax.me/raster"

    private static func render(_ coordinate: CLLocationCoordinate2D, frame: RadarFrame, palette: String, server: String) async throws -> UIImage {
        let zoom = min(max(frame.maxZoom ?? 7, 1), 7)
        let size = CGSize(width: 256, height: 190)
        // Web-mercator pixel space at `zoom` with 256-pt tiles; the image is centred on the fix.
        let center = worldPoint(coordinate, zoom: zoom)
        let origin = CGPoint(x: center.x - size.width / 2, y: center.y - size.height / 2)

        // The basemap is context, not data: if it fails the radar still renders over a plain ground.
        // Static-map zoom uses MapLibre's 512-px convention, one less than the 256-px tile zoom.
        let basemapURL = URL(string: String(format: "%@/styles/dark/static/%.5f,%.5f,%d/%dx%d@2x.png",
                                            basemap, coordinate.longitude, coordinate.latitude, zoom - 1,
                                            Int(size.width), Int(size.height)))
        async let basemapImage: UIImage? = {
            guard let basemapURL, let data = try? await fetch(basemapURL) else { return nil }
            return UIImage(data: data)
        }()

        // At this viewport size at most four tiles intersect the image.
        var requests: [(Int, Int, Task<Data?, Error>)] = []
        for x in Int(floor(origin.x / 256))...Int(floor((origin.x + size.width) / 256)) {
            for y in Int(floor(origin.y / 256))...Int(floor((origin.y + size.height) / 256)) {
                guard let url = frame.tileURL(server: server, palette: palette, z: zoom, x: x, y: y) else { throw SnapshotError.unavailable }
                requests.append((x, y, Task { try await fetchTile(url) }))
            }
        }
        defer { for (_, _, request) in requests { request.cancel() } }
        var tiles: [(UIImage, CGRect)] = []
        for (x, y, request) in requests {
            guard let data = try await request.value else { continue }  // no precipitation in this tile
            guard let image = UIImage(data: data) else { throw SnapshotError.unavailable }
            tiles.append((image, CGRect(x: CGFloat(x) * 256 - origin.x, y: CGFloat(y) * 256 - origin.y,
                                        width: 256, height: 256)))
        }
        let base = await basemapImage

        let format = UIGraphicsImageRendererFormat()
        format.scale = 2
        return UIGraphicsImageRenderer(size: size, format: format).image { context in
            if let base {
                base.draw(in: CGRect(origin: .zero, size: size))
            } else {
                UIColor(red: 0.11, green: 0.13, blue: 0.15, alpha: 1).setFill()
                context.fill(CGRect(origin: .zero, size: size))
            }
            for (image, tileRect) in tiles { image.draw(in: tileRect, blendMode: .normal, alpha: 0.75) }
            let dot = CGRect(x: size.width / 2 - 3, y: size.height / 2 - 3, width: 6, height: 6)
            UIColor.white.setFill()
            context.cgContext.fillEllipse(in: dot.insetBy(dx: -2, dy: -2))
            UIColor.systemBlue.setFill()
            context.cgContext.fillEllipse(in: dot)
            if base != nil {
                // OpenStreetMap's licence requires visible attribution on the rendered map.
                let credit = NSAttributedString(string: "© OpenStreetMap", attributes: [
                    .font: UIFont.systemFont(ofSize: 7, weight: .medium),
                    .foregroundColor: UIColor.white.withAlphaComponent(0.7),
                ])
                let textSize = credit.size()
                credit.draw(at: CGPoint(x: size.width - textSize.width - 4, y: size.height - textSize.height - 2))
            }
        }
    }

    /// Web-mercator position of `coordinate` in points at `zoom`, with 256-pt tiles.
    static func worldPoint(_ coordinate: CLLocationCoordinate2D, zoom: Int) -> CGPoint {
        let scale = 256 * Double(1 << zoom)
        let latitude = min(85.0511, max(-85.0511, coordinate.latitude)) * .pi / 180
        return CGPoint(x: (coordinate.longitude + 180) / 360 * scale,
                       y: (1 - log(tan(latitude) + 1 / cos(latitude)) / .pi) / 2 * scale)
    }

    private enum SnapshotError: Error { case unavailable }
}

@MainActor
private final class WidgetLocation: NSObject, @preconcurrency CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private var continuation: CheckedContinuation<CLLocation?, Never>?
    var isAuthorized: Bool { manager.isAuthorizedForWidgetUpdates }

    func locate() async -> CLLocation? {
        guard manager.isAuthorizedForWidgetUpdates else { return nil }
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyKilometer
        return await withCheckedContinuation { continuation in
            self.continuation = continuation
            manager.requestLocation()
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: .seconds(4))
                self?.finish(nil)
            }
        }
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        finish(locations.last(where: { $0.horizontalAccuracy >= 0 && abs($0.timestamp.timeIntervalSinceNow) < 300 }))
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) { finish(nil) }

    private func finish(_ location: CLLocation?) {
        continuation?.resume(returning: location)
        continuation = nil
        manager.stopUpdatingLocation()
    }
}
