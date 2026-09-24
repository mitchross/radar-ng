import CoreLocation
import UIKit

@MainActor
enum RadarSnapshot {
    private static var server: String { RadarShared.serverURL }

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
        if let last = shared?.location {
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
            let manifest = try JSONDecoder().decode(Manifest.self,
                from: await fetch(URL(string: "\(server)/api/manifest.json")!))
            guard let layer = manifest.layers["radar"],
                  let frame = layer.frames?.max(by: { $0.timestamp < $1.timestamp }),
                  let observedAt = parseDate(frame.timestamp) else { throw SnapshotError.unavailable }
            let palette = RadarShared.palette(among: frame.palettes ?? layer.palettes ?? [])
            let image = try await render(coordinate, frame: frame, palette: palette)
            return RadarEntry(date: .now, image: image, observedAt: observedAt,
                              message: Date().timeIntervalSince(observedAt) > 900 ? "Older radar" : "Nearby · Snapshot")
        } catch {
            // Missing tiles must not masquerade as dry conditions.
            return RadarEntry(date: .now, message: "Radar unavailable · Try later")
        }
    }

    private static let fractionalISO: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()
    private static let plainISO = ISO8601DateFormatter()

    static func parseDate(_ value: String) -> Date? {
        fractionalISO.date(from: value) ?? plainISO.date(from: value)
    }

    private static func fetch(_ url: URL) async throws -> Data {
        var request = URLRequest(url: url)
        request.timeoutInterval = 8
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw SnapshotError.unavailable
        }
        return data
    }

    /// Self-hosted basemap renderer (tileserver-gl over VersaTiles). Same style family as the phone.
    private static let basemap = "https://maps.vanillax.me/raster"

    private static func render(_ coordinate: CLLocationCoordinate2D, frame: Frame, palette: String) async throws -> UIImage {
        let zoom = min(max(frame.max_zoom ?? 7, 1), 7)
        let count = 1 << zoom
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
        var requests: [(Int, Int, Task<Data, Error>)] = []
        for x in Int(floor(origin.x / 256))...Int(floor((origin.x + size.width) / 256)) {
            for y in Int(floor(origin.y / 256))...Int(floor((origin.y + size.height) / 256)) {
                guard (0..<count).contains(x), (0..<count).contains(y),
                      let url = URL(string: "\(server)/tiles/radar/\(palette)/\(frame.path)/\(zoom)/\(x)/\(y).png") else { throw SnapshotError.unavailable }
                requests.append((x, y, Task { try await fetch(url) }))
            }
        }
        defer { for (_, _, request) in requests { request.cancel() } }
        var tiles: [(UIImage, CGRect)] = []
        for (x, y, request) in requests {
            guard let image = UIImage(data: try await request.value) else { throw SnapshotError.unavailable }
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

    private struct Manifest: Decodable { let layers: [String: Layer] }
    private struct Layer: Decodable { let frames: [Frame]?; let palettes: [String]? }
    private struct Frame: Decodable {
        let timestamp: String
        let path: String
        let palettes: [String]?
        let max_zoom: Int?
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
