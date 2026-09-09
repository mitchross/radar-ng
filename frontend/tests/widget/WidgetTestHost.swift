import SwiftUI
import MapKit

/// Isolated simulator host for real network/MapKit rendering and small-size layout.
/// Never compiled into the shipping app or extension.
@main
struct WidgetTestHost: App {
    @State private var entry = RadarEntry(date: .now, message: "Loading test snapshot")
    @State private var result = "Running"

    var body: some Scene {
        WindowGroup {
            VStack(spacing: 18) {
                Text("Radar widget verification").font(.headline)
                Text(result).accessibilityIdentifier("widget-test-result")
                RadarWidgetView(entry: entry)
                    .frame(width: 160, height: 170).padding(16)
                    .background(.background, in: RoundedRectangle(cornerRadius: 22))
                HStack(spacing: 10) {
                    RadarWidgetView(entry: RadarEntry(date: .now,
                        message: "Open Radar NG to enable location", symbol: "location.slash"))
                    RadarWidgetView(entry: RadarEntry(date: .now,
                        message: "Radar unavailable · Try later"))
                }
                .frame(height: 160).padding()
                Text("Real radar • Grand Rapids test coordinate")
                    .font(.caption)
            }
            .padding().preferredColorScheme(.dark)
            .task {
                precondition(RadarSnapshot.parseDate("2026-09-09T01:10:05+00:00") != nil)
                precondition(RadarSnapshot.parseDate("2026-09-09T01:10:05.123Z") != nil)
                precondition(RadarSnapshot.parseDate("invalid") == nil)
                let outside = await RadarSnapshot.load(at: CLLocationCoordinate2D(latitude: 51.5, longitude: 0))
                precondition(outside.image == nil)
                let start = Date()
                entry = await RadarSnapshot.load(at: CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681))
                let passed = entry.image != nil && entry.observedAt != nil
                result = passed ? "PASS · \(String(format: "%.2f", Date().timeIntervalSince(start)))s" : "FAIL · \(entry.message)"
                let directory = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
                try? result.write(to: directory.appendingPathComponent("result.txt"), atomically: true, encoding: .utf8)
                if let image = entry.image { try? image.pngData()?.write(to: directory.appendingPathComponent("snapshot.png")) }
            }
        }
    }
}
