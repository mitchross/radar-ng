import SwiftUI
import CoreLocation

/// Self-hosted MRMS radar over a native Apple map snapshot, centered on the user.
/// watchOS has no MKTileOverlay, so slippy-map tiles are composited by hand:
/// only visible radar tiles are layered over the snapshot, offset so the
/// user's location sits at screen center.
struct RadarMapView: View {
    @EnvironmentObject var store: WatchStore
    @State private var zoom: Double = 7

    private let fallback = CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681)
    private let tileSize: CGFloat = 256

    var body: some View {
        let coord = store.location ?? fallback
        let maxZoom = store.radarFrame?.maxZoom ?? 7
        let z = min(Int(zoom.rounded()), maxZoom)
        let n = 1 << z
        let xf = Double(n) * (coord.longitude + 180) / 360
        let latRad = coord.latitude * .pi / 180
        let yf = Double(n) * (1 - log(tan(latRad) + 1 / cos(latRad)) / .pi) / 2

        GeometryReader { geo in
            let center = CGPoint(x: geo.size.width / 2, y: geo.size.height / 2)
            ZStack {
                WatchBasemapView(latitude: coord.latitude, longitude: coord.longitude, zoom: z, size: geo.size)
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
                            if let url = store.radarFrame?.tileURL(z: z, x: wx, y: ty) {
                                TileImage(url: url)
                                    .opacity(0.82)
                                    .position(pos)
                            }
                            }
                        }
                    }
                }
                Circle()
                    .fill(.blue)
                    .frame(width: 9, height: 9)
                    .overlay(Circle().stroke(.white, lineWidth: 1.5))
            }
            .frame(width: geo.size.width, height: geo.size.height)
            .clipped()
        }
        .ignoresSafeArea()
        .focusable()
        .digitalCrownRotation($zoom, from: 4, through: Double(store.radarFrame?.maxZoom ?? 7), by: 1, sensitivity: .low)
        .overlay(alignment: .top) { radarStatus }
        .overlay(alignment: .bottom) { controls }
        .onChange(of: store.radarFrame?.maxZoom) { _, maxZoom in
            zoom = min(zoom, Double(maxZoom ?? 7))
        }
    }

    @ViewBuilder
    private var radarStatus: some View {
        if let frame = store.radarFrame {
            Label(store.radarErrorMessage == nil ? Self.ageLabel(frame.timestamp) : "Offline · saved radar", systemImage: "dot.radiowaves.left.and.right")
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
            Button { zoom = max(4, zoom - 1) } label: {
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

    private static func ageLabel(_ timestamp: String) -> String {
        guard let date = ISO8601DateFormatter().date(from: timestamp) else { return "Radar time unknown" }
        let minutes = max(0, Int(Date().timeIntervalSince(date) / 60))
        return minutes < 1 ? "Radar now" : "Radar \(minutes)m ago"
    }
}

private struct TileImage: View {
    let url: URL

    var body: some View {
        AsyncImage(url: url) { phase in
            if let image = phase.image {
                image
                    .resizable()
                    .frame(width: 256, height: 256)
            } else {
                Color.clear.frame(width: 256, height: 256)
            }
        }
        .accessibilityHidden(true)
    }
}
