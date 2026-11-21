import Foundation

/// CarPlay map rules with no UIKit or MapKit, so tests/carplay/MapViewportChecks.swift can run them.
enum RadarMapViewport {
    /// MKMapSize.world.width, in map points.
    static let worldMapPoints = 268_435_456.0
    /// Earth's circumference at the equator, in meters (Web Mercator).
    static let equatorMeters = 40_075_016.686
    /// Closest view: a few streets across.
    static let minWidthMeters = 1_000.0
    /// Widest view: one region, where radar still reads (about z5 on an 800 pt head unit).
    static let maxWidthMeters = 2_000_000.0
    /// The server has radar at z4-7; never let MapKit pick a tile zoom below z4 (half a level of margin).
    static let minTileZoom = 4.5
    /// One map zoom level per +/- press.
    static let zoomStep = 1.0
    /// Fraction of the view one arrow press moves.
    static let panStep = 0.15
    /// Navigation keeps the street span the trip starts with.
    static let navigationSpanMeters = 2_400.0

    /// The span the locate button returns to: the first-fix regional view, or the navigation view.
    static func resetSpanMeters(navigating: Bool, dashboard: Bool) -> Double {
        navigating ? navigationSpanMeters : (dashboard ? 140_000 : 180_000)
    }

    /// Ground meters across one map point at this latitude.
    static func metersPerMapPoint(latitude: Double) -> Double {
        equatorMeters * cos(latitude * .pi / 180) / worldMapPoints
    }

    /// The widest view in meters: the regional cap, or narrower if a small screen would drop below z4.
    static func maxWidthMeters(viewWidthPoints: Double, latitude: Double) -> Double {
        guard viewWidthPoints > 0 else { return maxWidthMeters }
        let atMinZoom = viewWidthPoints / (256 * pow(2, minTileZoom)) * equatorMeters * cos(latitude * .pi / 180)
        return max(minWidthMeters, min(maxWidthMeters, atMinZoom))
    }

    static func clampedWidth(_ meters: Double, viewWidthPoints: Double, latitude: Double) -> Double {
        min(maxWidthMeters(viewWidthPoints: viewWidthPoints, latitude: latitude), max(minWidthMeters, meters))
    }

    /// Positive levels zoom in; each level halves the width.
    static func zoomedWidth(_ meters: Double, levels: Double, viewWidthPoints: Double, latitude: Double) -> Double {
        clampedWidth(meters / pow(2, levels), viewWidthPoints: viewWidthPoints, latitude: latitude)
    }

    /// How far a released pan coasts, in screen points: an exponential decay, capped to part of the view.
    static func momentum(velocityX: Double, velocityY: Double, viewWidth: Double, viewHeight: Double)
        -> (dx: Double, dy: Double)
    {
        let speed = (velocityX * velocityX + velocityY * velocityY).squareRoot()
        guard speed >= 150 else { return (0, 0) }  // a slow release just stops
        let tau = 0.25  // seconds; distance = v * tau
        func cap(_ value: Double, _ limit: Double) -> Double { max(-limit, min(limit, value)) }
        return (cap(velocityX * tau, viewWidth * 0.6), cap(velocityY * tau, viewHeight * 0.6))
    }

    struct Tile: Hashable { let z: Int; let x: Int; let y: Int }

    /// Statuses of the tiles covering this map-point rect, at the closest zoom loaded there.
    static func visibleStatuses(_ tiles: [Tile: RadarTileStatus],
                                minX: Double, minY: Double, maxX: Double, maxY: Double) -> [RadarTileStatus] {
        let hits = tiles.filter { tile, _ in
            let size = worldMapPoints / Double(1 << tile.z)
            let x = Double(tile.x) * size, y = Double(tile.y) * size
            return x < maxX && x + size > minX && y < maxY && y + size > minY
        }
        guard let closest = hits.keys.map(\.z).max() else { return [] }
        return hits.filter { $0.key.z == closest }.map(\.value)
    }

    /// The short status button text.
    static func statusText(frameAge: TimeInterval?, fetchFailed: Bool, hidden: Bool,
                           tiles: [RadarTileStatus]) -> String {
        guard let age = frameAge else { return fetchFailed ? "Radar offline" : "Loading radar" }
        if age > RadarFrame.freshSeconds { return "Radar expired" }
        if hidden { return "Radar hidden" }
        if tiles.contains(.failure) { return tiles.contains(.image) ? "Radar incomplete" : "Radar offline" }
        if !tiles.isEmpty && tiles.allSatisfy({ $0 == .clear }) { return "No rain nearby" }
        return "Radar \(max(0, Int(age / 60)))m"
    }

    /// The parent-tile pixel square one overzoomed child shows; at least one pixel.
    static func overzoomCrop(childX: Int, childY: Int, factor: Int, sourceSize: Int)
        -> (x: Int, y: Int, size: Int)
    {
        let factor = max(1, factor)
        let size = max(1, sourceSize / factor)
        let x = min(sourceSize - size, (childX % factor) * sourceSize / factor)
        let y = min(sourceSize - size, (childY % factor) * sourceSize / factor)
        return (x, y, size)
    }
}
