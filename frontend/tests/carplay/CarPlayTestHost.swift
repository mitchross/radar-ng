import SwiftUI
import MapKit

/// Development preview: validates MapKit/navigation without claiming that the
/// CarPlay entitlement or a physical vehicle has been tested.
@main
struct CarPlayTestHost: App {
    var body: some Scene { WindowGroup { DrivingPreview() } }
}

@MainActor
private final class PreviewModel: ObservableObject {
    let driving = RadarDrivingSession(usesLiveLocation: false)
    @Published var title = "Searching Grand Rapids"
    @Published var detail = "Real MapKit routing · Radar NG precipitation"
    @Published var instruction = "Preparing route"
    @Published var status = "Running tests"
    @Published var places: [MKMapItem] = []
    @Published var stage = 0
    private var observer: UUID?
    private let directory = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]

    func run() async {
        do {
            driving.muted = true
            observer = driving.observe { [weak self] in
                guard let self else { return }
                self.detail = self.driving.radarMessage
                let index = self.driving.nextStepIndex
                if index < self.driving.steps.count {
                    self.instruction = self.driving.steps[index].instructions
                }
            }
            let start = CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681)
            func fix(_ coordinate: CLLocationCoordinate2D, age: Double = 0) -> CLLocation {
                CLLocation(coordinate: coordinate, altitude: 0, horizontalAccuracy: 5,
                           verticalAccuracy: 5, course: 0, speed: 12,
                           timestamp: Date().addingTimeInterval(-age))
            }
            driving.receive(fix(start))
            let search = MKLocalSearch.Request()
            search.naturalLanguageQuery = "Gerald R Ford International Airport"
            search.region = MKCoordinateRegion(center: start, latitudinalMeters: 80_000, longitudinalMeters: 80_000)
            places = Array(try await MKLocalSearch(request: search).start().mapItems.prefix(5))
            guard let destination = places.first else { throw PreviewError("MapKit returned no airport") }
            title = "Find a destination"
            stage = 0
            try await Task.sleep(for: .seconds(3))
            saveScreenshot("01-destination-search.jpg")
            driving.receive(fix(start))
            try await driving.preview(destination: destination)
            guard let route = driving.selectedRoute else { throw PreviewError("Route missing") }
            guard driving.steps.count > 1, driving.stepOffsets == driving.stepOffsets.sorted() else {
                throw PreviewError("Maneuver sequence is invalid")
            }
            title = destination.name ?? "Driving route"
            stage = 1
            instruction = "\(Int(ceil(route.expectedTravelTime / 60))) min · \(MKDistanceFormatter().string(fromDistance: route.distance))"
            try await Task.sleep(for: .seconds(4))
            saveScreenshot("02-route-preview.jpg")
            driving.receive(fix(start))
            try driving.startNavigation()
            guard driving.phase == .navigating else { throw PreviewError("Navigation did not start") }
            let geometry = RadarRouteProgress(polyline: route.polyline)
            // Drive the actual route geometry with simulated GPS, checking that
            // instructions advance and poor fixes cannot advance the journey.
            var advances = 0
            var previous = driving.nextStepIndex
            for (index, point) in geometry.points.enumerated() {
                if geometry.cumulative[index] > 2200 { break }
                driving.receive(fix(point.coordinate))
                if driving.nextStepIndex > previous { advances += 1; previous = driving.nextStepIndex }
            }
            let progress = driving.progress
            let beforeInvalid = driving.nextStepIndex
            driving.receive(fix(geometry.points.last!.coordinate, age: 60))
            guard driving.progress == progress, driving.nextStepIndex == beforeInvalid else {
                throw PreviewError("Stale GPS advanced navigation")
            }
            guard progress > 100, advances > 0 else { throw PreviewError("Route progression failed") }
            stage = 2
            title = "Turn-by-turn route guidance"
            try await Task.sleep(for: .seconds(4))
            saveScreenshot("03-navigation-preview.jpg")
            driving.endNavigation()
            guard driving.phase == .browsing, driving.selectedRoute == nil else {
                throw PreviewError("Navigation cancellation failed")
            }
            let frame = try await RadarAPI.latestFrame()
            guard frame.isValid else { throw PreviewError("Invalid radar frame") }
            let overlay = RadarTileOverlay(frame: frame)
            let point = MKMapPoint(start)
            let z = 7, count = Double(1 << z)
            let tile = MKTileOverlayPath(x: Int(point.x / MKMapSize.world.width * count),
                                        y: Int(point.y / MKMapSize.world.height * count), z: z, contentScaleFactor: 1)
            let data: Data = try await withCheckedThrowingContinuation { continuation in
                overlay.loadTile(at: tile) { data, error in
                    if let data { continuation.resume(returning: data) }
                    else { continuation.resume(throwing: error ?? PreviewError("Radar tile missing")) }
                }
            }
            guard UIImage(data: data) != nil else { throw PreviewError("Radar PNG invalid") }
            status = "PASS: search, routes, maneuver progression, stale GPS, cancel, live radar"
            try status.write(to: directory.appendingPathComponent("result.txt"), atomically: true, encoding: .utf8)
        } catch {
            status = "FAIL: \(error.localizedDescription)"
            try? status.write(to: directory.appendingPathComponent("result.txt"), atomically: true, encoding: .utf8)
        }
    }

    private func saveScreenshot(_ name: String) {
        guard let window = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
            .flatMap(\.windows).first(where: \.isKeyWindow) else { return }
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let image = UIGraphicsImageRenderer(bounds: window.bounds, format: format).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        try? image.jpegData(compressionQuality: 0.85)?.write(to: directory.appendingPathComponent(name))
    }

    private struct PreviewError: LocalizedError {
        let errorDescription: String?
        init(_ message: String) { errorDescription = message }
    }
}

private struct MapPreview: UIViewControllerRepresentable {
    let driving: RadarDrivingSession
    func makeUIViewController(context: Context) -> RadarMapController {
        let controller = RadarMapController(dashboard: true, driving: driving)
        controller.start()
        return controller
    }
    func updateUIViewController(_ controller: RadarMapController, context: Context) {}
    static func dismantleUIViewController(_ controller: RadarMapController, coordinator: ()) { controller.stop() }
}

private struct DrivingPreview: View {
    @StateObject private var model = PreviewModel()
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("RADAR NG · NAVIGATION").font(.headline)
            Text("Development preview · CarPlay approval pending")
                .font(.caption).foregroundStyle(.secondary)
            Text(model.title).font(.title2.bold())
            if model.stage == 0 {
                ForEach(Array(model.places.enumerated()), id: \.offset) { _, place in
                    VStack(alignment: .leading) {
                        Text(place.name ?? "Destination").font(.headline)
                        Text(place.addressRepresentations?.fullAddress(includingRegion: false, singleLine: true) ?? "")
                            .font(.caption).foregroundStyle(.secondary)
                    }.padding(.vertical, 4)
                }
            } else {
                Text(model.instruction).font(.headline)
            }
            MapPreview(driving: model.driving).clipShape(RoundedRectangle(cornerRadius: 16))
            Text(model.detail).font(.caption)
            Text("Simulated GPS · Real MapKit search and routes").font(.caption).foregroundStyle(.secondary)
            Text(model.status).font(.caption2).accessibilityIdentifier("carplay-test-result")
        }
        .padding().preferredColorScheme(.dark).task { await model.run() }
    }
}
