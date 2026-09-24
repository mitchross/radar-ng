import CoreLocation
import MapKit

@main
struct RouteProgressChecks {
    static func main() {
        precondition(RadarRouteProgress.roundaboutSymbol(instruction: "At the roundabout, turn left onto Peavine Rd") == "arrow.counterclockwise")
        precondition(RadarRouteProgress.roundaboutSymbol(instruction: "Turn right onto Roundabout Road") == nil)
        func route(_ coordinates: [CLLocationCoordinate2D]) -> RadarRouteProgress {
            RadarRouteProgress(polyline: MKPolyline(coordinates: coordinates, count: coordinates.count))
        }
        let a = CLLocationCoordinate2D(latitude: 42.96, longitude: -85.67)
        let b = CLLocationCoordinate2D(latitude: 42.97, longitude: -85.67)
        let c = CLLocationCoordinate2D(latitude: 42.97, longitude: -85.65)
        let line = route([a, b, c])
        let middle = CLLocationCoordinate2D(latitude: 42.965, longitude: -85.67)
        let projected = line.project(middle)!
        precondition(abs(projected.distance - CLLocation(latitude: a.latitude, longitude: a.longitude)
            .distance(from: CLLocation(latitude: middle.latitude, longitude: middle.longitude))) < 2)
        precondition(projected.offRoute < 1)
        let replay = line.coordinate(at: projected.distance)!
        precondition(CLLocation(latitude: replay.latitude, longitude: replay.longitude)
            .distance(from: CLLocation(latitude: middle.latitude, longitude: middle.longitude)) < 2)
        precondition(line.coordinate(at: -10)!.latitude == a.latitude)
        precondition(abs(line.coordinate(at: line.length + 10)!.longitude - c.longitude) < 0.000001)
        precondition(route([]).coordinate(at: 0) == nil)
        let north = MKPolyline(coordinates: [a, b], count: 2)
        let east = MKPolyline(coordinates: [b, c], count: 2)
        let west = MKPolyline(coordinates: [b, CLLocationCoordinate2D(latitude: b.latitude, longitude: b.longitude - 0.01)], count: 2)
        let ends = line.maneuverEndOffsets(for: [north, east])
        precondition(abs(ends[0] - RadarRouteProgress(polyline: north).length) < 1)
        precondition(abs(ends[1] - line.length) < 1)
        precondition(RadarRouteProgress.turnSymbol(incoming: north, outgoing: east) == "arrow.turn.up.right")
        precondition(RadarRouteProgress.turnSymbol(incoming: north, outgoing: west) == "arrow.turn.up.left")
        precondition(RadarRouteProgress.turnSymbol(incoming: east, outgoing: nil) == "flag.checkered")
        let loop = route([a, b, c, a])
        precondition(loop.project(a, near: 0)!.distance < 1)
        precondition(loop.project(c, near: 0, forwardWindow: 100)!.offRoute > 100)
        precondition(route([a, a]).project(a) == nil)
        let now = Date()
        func fix(age: TimeInterval, accuracy: Double) -> CLLocation {
            CLLocation(coordinate: a, altitude: 0, horizontalAccuracy: accuracy,
                       verticalAccuracy: 1, timestamp: now.addingTimeInterval(-age))
        }
        precondition(RadarRouteProgress.accepts(fix(age: 1, accuracy: 10), now: now))
        precondition(!RadarRouteProgress.accepts(fix(age: 30, accuracy: 10), now: now))
        precondition(!RadarRouteProgress.accepts(fix(age: 1, accuracy: -1), now: now))
        precondition(!RadarRouteProgress.accepts(fix(age: 1, accuracy: 100), now: now))
        let valid = RadarAPI.Frame(timestamp: ISO8601DateFormatter().string(from: now),
                                  path: "2026-09-21T04:04:06+00:00", palettes: ["classic"], max_zoom: 7)
        precondition(valid.isValid)
        let traversal = RadarAPI.Frame(timestamp: valid.timestamp, path: "../private", palettes: nil, max_zoom: nil)
        precondition(!traversal.isValid)
        let future = RadarAPI.Frame(timestamp: ISO8601DateFormatter().string(from: now.addingTimeInterval(600)),
                                   path: valid.path, palettes: nil, max_zoom: nil)
        precondition(!future.isValid)
        print("PASS: route projection, loop protection, GPS validity, radar frame validation")
    }
}
