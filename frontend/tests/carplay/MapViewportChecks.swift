import Foundation

@main
struct MapViewportChecks {
    static func main() {
        typealias V = RadarMapViewport
        // Reset returns to the first-fix span, or the street span while navigating.
        precondition(V.resetSpanMeters(navigating: false, dashboard: false) == 180_000)
        precondition(V.resetSpanMeters(navigating: false, dashboard: true) == 140_000)
        precondition(V.resetSpanMeters(navigating: true, dashboard: true) == 2_400)
        // The widest view never asks MapKit for tiles below z4, where the server has none.
        func tileZoom(widthMeters: Double, viewWidth: Double, latitude: Double) -> Double {
            log2(viewWidth * V.equatorMeters * cos(latitude * .pi / 180) / (widthMeters * 256))
        }
        for (view, lat) in [(800.0, 43.0), (800.0, 25.0), (300.0, 43.0), (1920.0, 48.0)] {
            let widest = V.clampedWidth(10_000_000, viewWidthPoints: view, latitude: lat)
            precondition(widest <= V.maxWidthMeters)
            precondition(tileZoom(widthMeters: widest, viewWidth: view, latitude: lat) >= 4.49)
        }
        precondition(V.clampedWidth(10_000_000, viewWidthPoints: 800, latitude: 43) == 2_000_000)
        precondition(V.clampedWidth(100, viewWidthPoints: 800, latitude: 43) == V.minWidthMeters)
        // One button press is one map zoom level.
        precondition(V.zoomedWidth(40_000, levels: 1, viewWidthPoints: 800, latitude: 43) == 20_000)
        precondition(V.zoomedWidth(40_000, levels: -1, viewWidthPoints: 800, latitude: 43) == 80_000)
        precondition(V.zoomedWidth(1_500_000, levels: -1, viewWidthPoints: 800, latitude: 43) == 2_000_000)
        // Momentum: a slow release stops; a fling coasts, capped to part of the screen.
        precondition(V.momentum(velocityX: 50, velocityY: 50, viewWidth: 800, viewHeight: 480) == (0, 0))
        let fling = V.momentum(velocityX: 800, velocityY: -10_000, viewWidth: 800, viewHeight: 480)
        precondition(fling.dx == 200 && fling.dy == -288)
        // Status reads the closest zoom loaded in view.
        let tileSize5 = V.worldMapPoints / 32
        let tiles: [V.Tile: RadarTileStatus] = [
            V.Tile(z: 5, x: 8, y: 11): .image,
            V.Tile(z: 6, x: 16, y: 22): .clear, V.Tile(z: 6, x: 17, y: 22): .clear,
            V.Tile(z: 6, x: 40, y: 40): .image,  // out of view
        ]
        let inView = V.visibleStatuses(tiles, minX: 8 * tileSize5 + 1, minY: 11 * tileSize5 + 1,
                                       maxX: 8.9 * tileSize5, maxY: 11.4 * tileSize5)
        precondition(inView.count == 2 && inView.allSatisfy { $0 == .clear })
        precondition(V.visibleStatuses(tiles, minX: 0, minY: 0, maxX: 1, maxY: 1).isEmpty)
        func status(_ age: TimeInterval?, failed: Bool = false, hidden: Bool = false,
                    _ tiles: [RadarTileStatus]) -> String {
            V.statusText(frameAge: age, fetchFailed: failed, hidden: hidden, tiles: tiles)
        }
        precondition(status(nil, []) == "Loading radar")
        precondition(status(nil, failed: true, []) == "Radar offline")
        precondition(status(130, [.clear, .clear]) == "No rain nearby")
        precondition(status(130, [.clear, .image]) == "Radar 2m")
        precondition(status(130, []) == "Radar 2m")
        precondition(status(130, [.failure, .clear]) == "Radar offline")
        precondition(status(130, [.failure, .image]) == "Radar incomplete")
        precondition(status(130, hidden: true, [.image]) == "Radar hidden")
        precondition(status(RadarFrame.freshSeconds + 1, [.image]) == "Radar expired")
        // Overzoom crops the child's square out of the parent: x % factor, y % factor, 256 / factor.
        precondition(V.overzoomCrop(childX: 5, childY: 3, factor: 1, sourceSize: 256) == (0, 0, 256))
        precondition(V.overzoomCrop(childX: 5, childY: 2, factor: 2, sourceSize: 256) == (128, 0, 128))
        precondition(V.overzoomCrop(childX: 33, childY: 63, factor: 32, sourceSize: 256) == (8, 248, 8))
        precondition(V.overzoomCrop(childX: 4095, childY: 0, factor: 4096, sourceSize: 256) == (255, 0, 1))
        for factor in [2, 8, 32, 512, 4096] {
            for child in [0, factor / 2, factor - 1] {
                let crop = V.overzoomCrop(childX: child, childY: child, factor: factor, sourceSize: 256)
                precondition(crop.size >= 1 && crop.x >= 0 && crop.x + crop.size <= 256)
            }
        }
        print("PASS: reset span, zoom clamp, momentum, radar status text, overzoom crop")
    }
}
