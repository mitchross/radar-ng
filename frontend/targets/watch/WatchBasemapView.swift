import SwiftUI

/// One self-hosted basemap image per viewport, reused when only the radar timestamp changes.
/// Rendered by the home cluster's tileserver-gl from the same VersaTiles styles the phone uses,
/// so the Watch never calls a third-party map service.
struct WatchBasemapView: View {
    let latitude: Double
    let longitude: Double
    let zoom: Int
    let size: CGSize
    @State private var image: UIImage?
    @State private var unavailable = false

    private var viewportKey: String { "\(latitude):\(longitude):\(zoom):\(size.width):\(size.height)" }

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
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
        .overlay(alignment: .bottomTrailing) {
            if image != nil {
                // OpenStreetMap's licence requires visible attribution on the rendered map.
                Text("© OpenStreetMap").font(.system(size: 7)).foregroundStyle(.white.opacity(0.7)).padding(2)
            }
        }
        .task(id: viewportKey) {
            guard size.width > 0, size.height > 0,
                  let url = WatchBasemap.url(latitude: latitude, longitude: longitude, zoom: zoom, size: size) else { return }
            image = nil
            unavailable = false
            var request = URLRequest(url: url)
            request.timeoutInterval = 10
            request.setValue("radar-ng/2.0 (watchOS)", forHTTPHeaderField: "User-Agent")
            do {
                let (data, response) = try await URLSession.shared.data(for: request)
                guard !Task.isCancelled else { return }
                guard (response as? HTTPURLResponse)?.statusCode == 200, let decoded = UIImage(data: data) else {
                    unavailable = true
                    return
                }
                image = decoded
            } catch {
                if !Task.isCancelled { unavailable = true }
            }
        }
    }
}
