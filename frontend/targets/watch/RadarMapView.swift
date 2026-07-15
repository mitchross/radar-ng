import SwiftUI
import CoreLocation

/// Live NEXRAD radar over a dark basemap, centered on the user.
/// watchOS has no MKTileOverlay, so slippy-map tiles are composited by hand:
/// a 3x3 grid of base tiles with radar tiles layered on top, offset so the
/// user's location sits at screen center.
struct RadarMapView: View {
    @EnvironmentObject var store: WatchStore
    @State private var zoom: Double = 7
    @State private var cacheKey = Int(Date().timeIntervalSince1970)

    private let fallback = CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681)
    private let tileSize: CGFloat = 256

    var body: some View {
        let coord = store.location ?? fallback
        let z = Int(zoom.rounded())
        let n = 1 << z
        let xf = Double(n) * (coord.longitude + 180) / 360
        let latRad = coord.latitude * .pi / 180
        let yf = Double(n) * (1 - log(tan(latRad) + 1 / cos(latRad)) / .pi) / 2

        GeometryReader { geo in
            let center = CGPoint(x: geo.size.width / 2, y: geo.size.height / 2)
            ZStack {
                Color.black
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
                            TileImage(url: Self.baseURL(z: z, x: wx, y: ty))
                                .position(pos)
                            TileImage(url: Self.radarURL(z: z, x: wx, y: ty, cacheKey: cacheKey))
                                .opacity(0.8)
                                .position(pos)
                        }
                    }
                }
                Circle()
                    .fill(.blue)
                    .frame(width: 9, height: 9)
                    .overlay(Circle().stroke(.white, lineWidth: 1.5))
            }
        }
        .ignoresSafeArea()
        .focusable()
        .digitalCrownRotation($zoom, from: 4, through: 10, by: 1, sensitivity: .low)
        .overlay(alignment: .bottom) { controls }
        .onTapGesture(count: 2) { cacheKey = Int(Date().timeIntervalSince1970) }
    }

    private var controls: some View {
        HStack {
            Button { zoom = max(4, zoom - 1) } label: {
                Image(systemName: "minus").font(.caption2.bold())
            }
            .buttonStyle(.plain)
            .frame(width: 28, height: 28)
            .background(.ultraThinMaterial, in: Circle())

            Spacer()

            Button { cacheKey = Int(Date().timeIntervalSince1970) } label: {
                Image(systemName: "arrow.clockwise").font(.caption2.bold())
            }
            .buttonStyle(.plain)
            .frame(width: 28, height: 28)
            .background(.ultraThinMaterial, in: Circle())

            Spacer()

            Button { zoom = min(10, zoom + 1) } label: {
                Image(systemName: "plus").font(.caption2.bold())
            }
            .buttonStyle(.plain)
            .frame(width: 28, height: 28)
            .background(.ultraThinMaterial, in: Circle())
        }
        .padding(.horizontal, 10)
        .padding(.bottom, 4)
    }

    private static func baseURL(z: Int, x: Int, y: Int) -> URL {
        let sub = ["a", "b", "c", "d"][(x + y) % 4]
        return URL(string: "https://\(sub).basemaps.cartocdn.com/dark_all/\(z)/\(x)/\(y)@2x.png")!
    }

    /// Same IEM NEXRAD source the CarPlay overlay uses; note the TMS y-flip.
    private static func radarURL(z: Int, x: Int, y: Int, cacheKey: Int) -> URL {
        let tmsY = (1 << z) - 1 - y
        return URL(string: "https://mesonet.agron.iastate.edu/cache/tile.py/1.0.0/nexrad-n0q-0/\(z)/\(x)/\(tmsY).png?c=\(cacheKey)")!
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
    }
}
