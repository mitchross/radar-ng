import SwiftUI
import MapKit

/// One native snapshot per viewport, reused when only the radar timestamp changes.
/// This avoids both API-key watermarks and downloading nine oversized base tiles.
struct WatchBasemapView: View {
    let latitude: Double
    let longitude: Double
    let zoom: Int
    let size: CGSize
    @State private var snapshot: MKMapSnapshotter.Snapshot?
    @State private var unavailable = false

    private var viewportKey: String { "\(latitude):\(longitude):\(zoom):\(size.width):\(size.height)" }

    var body: some View {
        Group {
            if let snapshot {
                Image(uiImage: snapshot.image)
                    .resizable()
                    .accessibilityIdentifier("watch-basemap")
            } else if unavailable {
                Color(white: 0.12)
                    .overlay { Text("Map unavailable").font(.caption2) }
            } else {
                Color(white: 0.12)
            }
        }
        .frame(width: size.width, height: size.height)
        .task(id: viewportKey) {
            guard size.width > 0, size.height > 0 else { return }
            snapshot = nil
            unavailable = false
            let point = MKMapPoint(CLLocationCoordinate2D(latitude: latitude, longitude: longitude))
            let unitsPerPoint = MKMapSize.world.width / (256 * Double(1 << zoom))
            let options = MKMapSnapshotter.Options()
            options.size = size
            options.mapRect = MKMapRect(
                x: point.x - Double(size.width) * unitsPerPoint / 2,
                y: point.y - Double(size.height) * unitsPerPoint / 2,
                width: Double(size.width) * unitsPerPoint,
                height: Double(size.height) * unitsPerPoint
            )
            let configuration = MKStandardMapConfiguration(elevationStyle: .flat, emphasisStyle: .muted)
            configuration.pointOfInterestFilter = .excludingAll
            options.preferredConfiguration = configuration
            let renderer = MKMapSnapshotter(options: options)
            do {
                let result = try await withTaskCancellationHandler {
                    try await renderer.start()
                } onCancel: {
                    renderer.cancel()
                }
                guard !Task.isCancelled else { return }
                snapshot = result
            } catch {
                if !Task.isCancelled { unavailable = true }
            }
        }
    }
}
