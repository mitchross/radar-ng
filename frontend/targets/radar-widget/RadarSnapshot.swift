import CoreLocation
import MapKit
import UIKit

@MainActor
enum RadarSnapshot {
    private static let server = "https://radar-ng-api.vanillax.me"

    static func load() async -> RadarEntry {
        let locator = WidgetLocation()
        guard locator.isAuthorized else {
            return RadarEntry(date: .now, message: "Open Radar NG to enable location", symbol: "location.slash")
        }
        guard let location = await locator.locate() else {
            return RadarEntry(date: .now, message: "Location unavailable · Try later", symbol: "location.slash")
        }
        return await load(at: location.coordinate)
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
            let palette = (frame.palettes ?? layer.palettes ?? []).contains("classic")
                ? "classic" : (frame.palettes ?? layer.palettes)?.first ?? "classic"
            let image = try await render(coordinate, frame: frame, palette: palette)
            return RadarEntry(date: .now, image: image, observedAt: observedAt,
                              message: Date().timeIntervalSince(observedAt) > 900 ? "Older radar" : "Nearby · Snapshot")
        } catch {
            // Missing tiles must not masquerade as dry conditions.
            return RadarEntry(date: .now, message: "Radar unavailable · Try later")
        }
    }

    static func parseDate(_ value: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.date(from: value) ?? ISO8601DateFormatter().date(from: value)
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

    private static func render(_ coordinate: CLLocationCoordinate2D, frame: Frame, palette: String) async throws -> UIImage {
        let zoom = min(max(frame.max_zoom ?? 7, 1), 7)
        let count = 1 << zoom
        let tileWidth = MKMapSize.world.width / Double(count)
        let point = MKMapPoint(coordinate)
        let rect = MKMapRect(x: point.x - tileWidth / 2, y: point.y - tileWidth / 2,
                             width: tileWidth, height: tileWidth)
        let size = CGSize(width: 256, height: 190)
        // Preserve the map's aspect ratio so radar registration is exact.
        let viewport = MKMapRect(x: rect.minX, y: point.y - tileWidth * 190 / 256 / 2,
                                 width: tileWidth, height: tileWidth * 190 / 256)
        let options = MKMapSnapshotter.Options()
        options.size = size
        options.scale = 1
        options.mapRect = viewport
        options.traitCollection = UITraitCollection(userInterfaceStyle: .dark)
        let configuration = MKStandardMapConfiguration(elevationStyle: .flat, emphasisStyle: .muted)
        configuration.pointOfInterestFilter = .excludingAll
        options.preferredConfiguration = configuration
        let snapshotter = MKMapSnapshotter(options: options)
        let timeout = Task { @MainActor in
            try? await Task.sleep(for: .seconds(8))
            if !Task.isCancelled { snapshotter.cancel() }
        }
        defer { timeout.cancel() }
        async let mapSnapshot = snapshotter.start()
        // At this viewport size at most four tiles intersect the image.
        var requests: [(Int, Int, Task<Data, Error>)] = []
        for x in Int(floor(viewport.minX / tileWidth))...Int(floor(viewport.maxX / tileWidth)) {
            for y in Int(floor(viewport.minY / tileWidth))...Int(floor(viewport.maxY / tileWidth)) {
                guard (0..<count).contains(x), (0..<count).contains(y),
                      let url = URL(string: "\(server)/tiles/radar/\(palette)/\(frame.path)/\(zoom)/\(x)/\(y).png") else { throw SnapshotError.unavailable }
                requests.append((x, y, Task { try await fetch(url) }))
            }
        }
        defer { for (_, _, request) in requests { request.cancel() } }
        let snapshot = try await mapSnapshot
        var tiles: [(UIImage, CGRect)] = []
        for (x, y, request) in requests {
                guard let image = UIImage(data: try await request.value) else { throw SnapshotError.unavailable }
                let origin = snapshot.point(for: MKMapPoint(x: Double(x) * tileWidth, y: Double(y) * tileWidth).coordinate)
                let end = snapshot.point(for: MKMapPoint(x: Double(x + 1) * tileWidth, y: Double(y + 1) * tileWidth).coordinate)
                tiles.append((image, CGRect(x: origin.x, y: origin.y, width: end.x - origin.x, height: end.y - origin.y)))
        }
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        return UIGraphicsImageRenderer(size: size, format: format).image { context in
            snapshot.image.draw(at: .zero)
            // Leave MapKit attribution at the bottom of the snapshot unobscured.
            context.cgContext.saveGState()
            context.cgContext.clip(to: CGRect(x: 0, y: 0, width: size.width, height: size.height - 22))
            for (image, tileRect) in tiles { image.draw(in: tileRect, blendMode: .normal, alpha: 0.75) }
            context.cgContext.restoreGState()
            let center = snapshot.point(for: coordinate)
            let dot = CGRect(x: center.x - 3, y: center.y - 3, width: 6, height: 6)
            UIColor.white.setFill()
            context.cgContext.fillEllipse(in: dot.insetBy(dx: -2, dy: -2))
            UIColor.systemBlue.setFill()
            context.cgContext.fillEllipse(in: dot)
        }
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
