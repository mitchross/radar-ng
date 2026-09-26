import SwiftUI
import CoreLocation

/// Self-hosted MRMS radar over a native Apple map snapshot, centered on the user.
/// watchOS has no MKTileOverlay, so slippy-map tiles are composited by hand:
/// only visible radar tiles are layered over the snapshot, offset so the
/// user's location sits at screen center.
struct RadarMapView: View {
    @EnvironmentObject var store: WatchStore
    @State private var zoom: Double = 7
    @State private var failedTiles: Set<String> = []

    private let fallback = CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681)
    private let tileSize: CGFloat = 256

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            radarMap(at: context.date)
        }
    }

    private func radarMap(at now: Date) -> some View {
        let coord = store.location ?? fallback
        let maxZoom = max(1, min(store.radarFrame?.maxZoom ?? 7, 7))
        let z = max(1, min(Int(zoom.rounded()), maxZoom))
        let n = 1 << z
        let xf = Double(n) * (coord.longitude + 180) / 360
        let latitude = min(85.0511, max(-85.0511, coord.latitude))
        let latRad = latitude * .pi / 180
        let covered = (20...55).contains(coord.latitude) && (-130 ... -60).contains(coord.longitude)
        let yf = Double(n) * (1 - log(tan(latRad) + 1 / cos(latRad)) / .pi) / 2

        return GeometryReader { geo in
            let center = CGPoint(x: geo.size.width / 2, y: geo.size.height / 2)
            ZStack {
                WatchBasemapView(latitude: latitude, longitude: coord.longitude, zoom: z, size: geo.size)
                ForEach(-1...1, id: \.self) { dy in
                    ForEach(-1...1, id: \.self) { dx in
                        let tx = Int(floor(xf)) + dx
                        let ty = Int(floor(yf)) + dy
                        if ty >= 0 && ty < n {
                            let wx = ((tx % n) + n) % n
                            let pos = CGPoint(
                                x: center.x + CGFloat(Double(tx) + 0.5 - xf) * tileSize,
                                y: center.y + CGFloat(Double(ty) + 0.5 - yf) * tileSize
                            )
                            if pos.x > -tileSize / 2 && pos.x < geo.size.width + tileSize / 2 &&
                                pos.y > -tileSize / 2 && pos.y < geo.size.height + tileSize / 2 {
                            if covered, let frame = store.radarFrame, frame.isFresh(at: now) {
                                TileImage(frame: frame, z: z, x: wx, y: ty, reload: store.radarRefreshID) { key, failed in
                                    if failed { failedTiles.insert(key) } else { failedTiles.remove(key) }
                                }
                                    .opacity(0.82)
                                    .position(pos)
                            }
                            }
                        }
                    }
                }
                if store.location != nil {
                Circle()
                    .fill(.blue)
                    .frame(width: 9, height: 9)
                    .overlay(Circle().stroke(.white, lineWidth: 1.5))
                }
            }
            .frame(width: geo.size.width, height: geo.size.height)
            .clipped()
        }
        .ignoresSafeArea()
        .focusable()
        .digitalCrownRotation($zoom, from: Double(min(4, maxZoom)), through: Double(maxZoom), by: 1, sensitivity: .low)
        .overlay(alignment: .top) {
            VStack(spacing: 2) {
                radarStatus(at: now, covered: covered)
                if let notice = store.locationNotice ?? (store.location == nil ? "Showing Grand Rapids" : nil) {
                    Text(notice).font(.system(size: 9)).padding(3).background(.ultraThinMaterial, in: Capsule())
                }
            }
        }
        .overlay(alignment: .bottom) { controls }
        .onChange(of: store.radarFrame?.maxZoom) { _, maxZoom in
            zoom = min(zoom, Double(maxZoom ?? 7))
        }
    }

    @ViewBuilder
    private func radarStatus(at now: Date, covered: Bool) -> some View {
        if !covered || store.radarFrame.map({ !$0.isFresh(at: now) }) == true {
            Text(!covered ? "Radar: continental US only" : "Radar expired · refresh")
                .font(.system(size: 9, weight: .semibold))
                .padding(6).background(.ultraThinMaterial, in: Capsule())
        } else if !failedTiles.isEmpty {
            Text("Radar incomplete · refresh").font(.system(size: 9, weight: .semibold))
                .padding(6).background(.ultraThinMaterial, in: Capsule())
        } else if let frame = store.radarFrame {
            Label(store.radarErrorMessage == nil ? Self.ageLabel(frame.timestamp, now: now) : "Offline · saved radar", systemImage: "dot.radiowaves.left.and.right")
                .font(.system(size: 9, weight: .semibold))
                .padding(.horizontal, 7)
                .padding(.vertical, 4)
                .background(.ultraThinMaterial, in: Capsule())
                .padding(.top, 3)
        } else if store.isRadarLoading {
            ProgressView()
                .controlSize(.mini)
                .padding(6)
                .background(.ultraThinMaterial, in: Circle())
                .padding(.top, 3)
        } else {
            Text(store.radarErrorMessage ?? "Radar unavailable")
                .font(.system(size: 9, weight: .semibold))
                .lineLimit(1)
                .padding(.horizontal, 7)
                .padding(.vertical, 4)
                .background(.ultraThinMaterial, in: Capsule())
                .padding(.top, 3)
        }
    }

    private var controls: some View {
        HStack {
            Button { zoom = max(Double(min(4, store.radarFrame?.maxZoom ?? 7)), zoom - 1) } label: {
                Image(systemName: "minus").font(.body.bold())
                    .frame(width: 36, height: 36).contentShape(Circle())
            }
            .accessibilityLabel("Zoom out")
            .accessibilityIdentifier("watch-zoom-out")
            .buttonStyle(.plain)
            .frame(width: 36, height: 36)
            .background(.ultraThinMaterial, in: Circle())

            Spacer()

            Button { Task { await store.refreshRadar() } } label: {
                Group {
                    if store.isRadarLoading {
                        ProgressView().controlSize(.mini)
                    } else {
                        Image(systemName: "arrow.clockwise").font(.body.bold())
                    }
                }
                .frame(width: 36, height: 36).contentShape(Circle())
            }
            .accessibilityLabel("Refresh radar")
            .accessibilityIdentifier("watch-radar-refresh")
            .buttonStyle(.plain)
            .frame(width: 36, height: 36)
            .background(.ultraThinMaterial, in: Circle())

            Spacer()

            Button { zoom = min(Double(store.radarFrame?.maxZoom ?? 7), zoom + 1) } label: {
                Image(systemName: "plus").font(.body.bold())
                    .frame(width: 36, height: 36).contentShape(Circle())
            }
            .accessibilityLabel("Zoom in")
            .accessibilityIdentifier("watch-zoom-in")
            .buttonStyle(.plain)
            .frame(width: 36, height: 36)
            .background(.ultraThinMaterial, in: Circle())
        }
        .padding(.horizontal, 10)
        .padding(.bottom, 4)
    }

    private static func ageLabel(_ timestamp: String, now: Date) -> String {
        guard let date = WatchRadarFrame.date(timestamp) else { return "Radar time unknown" }
        let minutes = max(0, Int(now.timeIntervalSince(date) / 60))
        return minutes < 1 ? "Radar now" : "Radar \(minutes)m ago"
    }
}

private struct TileImage: View {
    let frame: WatchRadarFrame
    let z: Int
    let x: Int
    let y: Int
    let reload: Int
    let reportFailure: (String, Bool) -> Void
    @State private var image: UIImage?
    @State private var failed = false
    @State private var reportedKey: String?
    private var requestKey: String { "\(frame.path):\(frame.palette):\(z):\(x):\(y):\(reload)" }

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image).resizable()
            } else if failed {
                Color.black.opacity(0.55).overlay {
                    Label("Radar unavailable", systemImage: "wifi.slash")
                        .font(.caption2).padding(8)
                }
            } else {
                Color.black.opacity(0.25).overlay { ProgressView().controlSize(.mini) }
            }
        }
        .frame(width: 256, height: 256)
        .onDisappear { if let reportedKey { reportFailure(reportedKey, false) } }
        .task(id: requestKey) {
            if let reportedKey { reportFailure(reportedKey, false) }
            reportedKey = requestKey
            image = nil
            failed = false
            // The tiler writes no file for a tile without precipitation, so 404
            // means clear. A 404 can also mean the zoom exceeds the product's
            // coverage, so try the same frame's parent tiles first; only when
            // every level is 404 is the area clear. Other failures still report.
            var onlyNotFound = true
            for level in stride(from: z, through: min(4, z), by: -1) {
                guard !Task.isCancelled else { return }
                let factor = 1 << (z - level)
                guard let url = frame.tileURL(z: level, x: x / factor, y: y / factor) else { continue }
                do {
                    var request = URLRequest(url: url)
                    request.timeoutInterval = 8
                    let (data, response) = try await URLSession.shared.data(for: request)
                    guard !Task.isCancelled else { return }
                    let status = (response as? HTTPURLResponse)?.statusCode
                    if status != 404 { onlyNotFound = false }
                    guard status == 200, let decoded = UIImage(data: data)?.cgImage else { continue }
                    let size = CGFloat(decoded.width) / CGFloat(factor)
                    let rect = CGRect(x: CGFloat(x % factor) * size, y: CGFloat(y % factor) * size,
                                      width: size, height: size)
                    if let crop = decoded.cropping(to: rect) {
                        image = UIImage(cgImage: crop)
                        return
                    }
                } catch {
                    if Task.isCancelled { return }
                    onlyNotFound = false
                }
            }
            if onlyNotFound { return }  // clear: no precipitation at any level
            failed = true
            reportFailure(requestKey, true)
        }
    }
}
