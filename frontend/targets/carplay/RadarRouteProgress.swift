import CoreLocation
import MapKit

/// Match only the nearby section of a route to avoid jumping to a later crossing.
struct RadarRouteProgress {
    struct Projection {
        let distance: CLLocationDistance
        let offRoute: CLLocationDistance
    }
    let points: [MKMapPoint]
    let cumulative: [CLLocationDistance]
    var length: CLLocationDistance { cumulative.last ?? 0 }

    /// Interpolate a route position for deterministic, clearly labelled GPS replay.
    func coordinate(at distance: CLLocationDistance) -> CLLocationCoordinate2D? {
        guard let first = points.first else { return nil }
        let distance = min(length, max(0, distance))
        for i in 1..<points.count where cumulative[i] > cumulative[i - 1] && cumulative[i] >= distance {
            let fraction = (distance - cumulative[i - 1]) / (cumulative[i] - cumulative[i - 1])
            return MKMapPoint(x: points[i - 1].x + (points[i].x - points[i - 1].x) * fraction,
                              y: points[i - 1].y + (points[i].y - points[i - 1].y) * fraction).coordinate
        }
        return points.last?.coordinate ?? first.coordinate
    }

    init(polyline: MKPolyline) {
        points = Array(UnsafeBufferPointer(start: polyline.points(), count: polyline.pointCount))
        var lengths = [CLLocationDistance](repeating: 0, count: points.count)
        for index in 1..<max(1, points.count) {
            lengths[index] = lengths[index - 1] + points[index - 1].distance(to: points[index])
        }
        cumulative = lengths
    }

    func project(_ coordinate: CLLocationCoordinate2D,
                 near progress: CLLocationDistance? = nil,
                 forwardWindow: CLLocationDistance = 400) -> Projection? {
        guard points.count > 1 else { return nil }
        let position = MKMapPoint(coordinate)
        var best: Projection?
        for i in 1..<points.count {
            if let progress,
               (cumulative[i] < progress - 60 || cumulative[i - 1] > progress + forwardWindow) { continue }
            let a = points[i - 1], b = points[i]
            let dx = b.x - a.x, dy = b.y - a.y
            let squaredLength = dx * dx + dy * dy
            guard squaredLength > 0 else { continue }
            let segmentLength = cumulative[i] - cumulative[i - 1]
            guard segmentLength > 0 else { continue }
            let lower = progress.map { max(0, ($0 - 60 - cumulative[i - 1]) / segmentLength) } ?? 0
            let upper = progress.map { min(1, ($0 + forwardWindow - cumulative[i - 1]) / segmentLength) } ?? 1
            let fraction = min(upper, max(lower, ((position.x - a.x) * dx + (position.y - a.y) * dy) / squaredLength))
            let projected = MKMapPoint(x: a.x + dx * fraction, y: a.y + dy * fraction)
            let candidate = Projection(distance: cumulative[i - 1] + fraction * (cumulative[i] - cumulative[i - 1]),
                                       offRoute: position.distance(to: projected))
            if best == nil || candidate.offRoute < best!.offRoute { best = candidate }
        }
        return best
    }

    /// MapKit step instructions refer to the maneuver at the segment end.
    func maneuverEndOffsets(for segments: [MKPolyline]) -> [CLLocationDistance] {
        var expected: CLLocationDistance = 0
        return segments.map { polyline in
            let segment = RadarRouteProgress(polyline: polyline)
            expected += segment.length
            let projection = segment.points.last.flatMap {
                project($0.coordinate, near: expected, forwardWindow: 100)
            }
            expected = max(expected - segment.length, projection?.distance ?? expected)
            return expected
        }
    }

    static func turnSymbol(incoming: MKPolyline, outgoing: MKPolyline?) -> String {
        guard let outgoing else { return "flag.checkered" }
        let before = RadarRouteProgress(polyline: incoming).points
        let after = RadarRouteProgress(polyline: outgoing).points
        guard before.count >= 2, after.count >= 2 else { return "mappin.and.ellipse" }
        let a = before[before.count - 2], b = before[before.count - 1]
        let c = after[0], d = after[1]
        var angle = (atan2(d.x - c.x, c.y - d.y) - atan2(b.x - a.x, a.y - b.y)) * 180 / .pi
        while angle > 180 { angle -= 360 }
        while angle < -180 { angle += 360 }
        if abs(angle) > 150 { return angle > 0 ? "arrow.uturn.right" : "arrow.uturn.left" }
        if angle > 35 { return "arrow.turn.up.right" }
        if angle < -35 { return "arrow.turn.up.left" }
        return "arrow.up"
    }

    /// MapKit can end a step at the roundabout entrance: the adjoining tangents
    /// then look straight even though the instruction describes a circular junction.
    /// Recognize this English instruction explicitly; do not match street names.
    /// This generic circulation symbol does not claim an exit number or geometry.
    static func roundaboutSymbol(instruction: String) -> String? {
        instruction.lowercased().hasPrefix("at the roundabout,") ? "arrow.counterclockwise" : nil
    }

    static func accepts(_ location: CLLocation, now: Date = .now) -> Bool {
        CLLocationCoordinate2DIsValid(location.coordinate) && location.horizontalAccuracy >= 0 &&
        location.horizontalAccuracy <= 65 && abs(location.timestamp.timeIntervalSince(now)) <= 10
    }
}
