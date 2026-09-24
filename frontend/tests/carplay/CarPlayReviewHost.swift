import SwiftUI
import CarPlay
import MapKit

/// Evidence for case 22361864. A development presentation of the SAME
/// CPManeuver objects used by RadarMapController, not a CarPlay system screenshot.
@main
struct CarPlayReviewHost: App {
    var body: some Scene { WindowGroup { ReviewScreen() } }
}

@MainActor
private final class ReviewModel: ObservableObject {
    let driving = RadarDrivingSession(usesLiveLocation: false)
    @Published var maneuver: CPManeuver?
    @Published var nextManeuver: CPManeuver?
    @Published var distance = "—"
    @Published var trip = "—"
    @Published var caption = "Preparing a driving route"
    @Published var radarStatus = "Loading radar"
    @Published var index = 0
    @Published var status = "Preparing evidence"
    private let environment = ProcessInfo.processInfo.environment
    var city: String { environment["RADAR_REVIEW_CITY"] ?? "Grand Rapids" }
    var mapSpan: Double { Double(environment["RADAR_REVIEW_MAP_METERS"] ?? "2200") ?? 2200 }
    private var requiresRadar: Bool { environment["RADAR_REVIEW_REQUIRE_RADAR"] == "1" }
    private var loadedTiles: [String: Int] = [:]
    private var loadedFrame: RadarAPI.Frame?
    private var maneuvers: [CPManeuver] = []
    private var observation: UUID?
    private var records: [[String: Any]] = []
    private let directory = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        .appendingPathComponent(ProcessInfo.processInfo.environment["RADAR_REVIEW_RUN_ID"] ?? "manual")

    func run() async {
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            for name in ["review-result.txt", "maneuver-evidence.json", "01-approaching-turn.jpg", "02-near-turn.jpg", "03-next-maneuver.jpg"] {
                try? FileManager.default.removeItem(at: directory.appendingPathComponent(name))
            }
            driving.muted = true
            observation = driving.observe { [weak self] in self?.sync() }
            let start = CLLocationCoordinate2D(
                latitude: Double(environment["RADAR_REVIEW_LATITUDE"] ?? "42.9634") ?? 42.9634,
                longitude: Double(environment["RADAR_REVIEW_LONGITUDE"] ?? "-85.6681") ?? -85.6681)
            guard CLLocationCoordinate2DIsValid(start), mapSpan.isFinite, mapSpan > 0 else {
                throw ReviewError("Invalid review map configuration")
            }
            driving.receive(fix(start))
            let search = MKLocalSearch.Request()
            search.naturalLanguageQuery = environment["RADAR_REVIEW_DESTINATION"] ?? "Gerald R Ford International Airport"
            search.region = MKCoordinateRegion(center: start, latitudinalMeters: 240_000, longitudinalMeters: 240_000)
            guard let destination = try await MKLocalSearch(request: search).start().mapItems.first else {
                throw ReviewError("MapKit returned no destination")
            }
            driving.receive(fix(start))
            try await driving.preview(destination: destination)
            guard let route = driving.selectedRoute else { throw ReviewError("No driving route") }
            maneuvers = RadarManeuverFactory.makeManeuvers(for: driving)
            guard maneuvers.count == driving.steps.count, maneuvers.count > 2,
                  maneuvers.allSatisfy({ !$0.instructionVariants.isEmpty && $0.symbolImage != nil && $0.initialTravelEstimates != nil }) else {
                throw ReviewError("Missing CPManeuver metadata")
            }
            let geometry = RadarRouteProgress(polyline: route.polyline)
            guard let target = driving.stepOffsets.indices.first(where: {
                driving.stepOffsets[$0] > 500 && driving.stepOffsets[$0] - ($0 > 0 ? driving.stepOffsets[$0 - 1] : 0) > 350
            }) else { throw ReviewError("No suitable maneuver for approach captures") }
            let turnDistance = driving.stepOffsets[target]
            driving.receive(fix(start))
            try driving.startNavigation()
            // Replay the real route in 25 m increments; no jumps across junctions.
            advance(to: turnDistance - 300, geometry: geometry)
            guard driving.nextStepIndex == target else { throw ReviewError("Approach points at wrong maneuver") }
            let far = driving.maneuverDistance
            caption = "1 · Approaching the next maneuver"
            try await capture("01-approaching-turn")
            advance(to: turnDistance - 40, geometry: geometry)
            guard driving.nextStepIndex == target, driving.maneuverDistance < far,
                  driving.maneuverDistance > 12 else { throw ReviewError("Distance did not decrease for the same maneuver") }
            caption = "2 · Same maneuver, updated distance"
            try await capture("02-near-turn")
            advance(to: turnDistance + 35, geometry: geometry)
            guard driving.nextStepIndex > target else { throw ReviewError("Next maneuver did not advance") }
            caption = "3 · Passed turn; next maneuver is active"
            try await capture("03-next-maneuver")
            let beforeInvalid = driving.progress
            driving.receive(fix(geometry.points.last!.coordinate, age: 60))
            guard driving.progress == beforeInvalid else { throw ReviewError("Stale GPS changed route progress") }
            driving.endNavigation()
            guard driving.phase == .browsing else { throw ReviewError("End trip failed") }
            let data = try JSONSerialization.data(withJSONObject: [
                "caseID": "22361864", "source": "Live MapKit route; replayed GPS; native CPManeuver objects",
                "presentation": "Development review host, not CarPlay system UI",
                "route": route.name, "city": city, "mapSpanMeters": mapSpan,
                "destination": destination.name ?? "", "snapshots": records,
                "radarTileCheckRequired": requiresRadar,
                "radarCheckScope": "Nontransparent tile data delivered to MapKit; screenshot appearance requires visual inspection",
                "checks": ["required maneuver metadata", "decreasing distance", "next maneuver advancement", "stale GPS rejected", "end trip"]
            ], options: [.prettyPrinted, .sortedKeys])
            try data.write(to: directory.appendingPathComponent("maneuver-evidence.json"))
            status = "PASS: CPManeuver metadata, approach updates, next turn, stale GPS, end trip"
            try status.write(to: directory.appendingPathComponent("review-result.txt"), atomically: true, encoding: .utf8)
        } catch {
            status = "FAIL: \(error.localizedDescription)"
            try? status.write(to: directory.appendingPathComponent("review-result.txt"), atomically: true, encoding: .utf8)
        }
    }

    private func fix(_ coordinate: CLLocationCoordinate2D, age: Double = 0) -> CLLocation {
        CLLocation(coordinate: coordinate, altitude: 0, horizontalAccuracy: 5, verticalAccuracy: 5,
                   course: 0, speed: 12, timestamp: Date().addingTimeInterval(-age))
    }

    private func advance(to distance: Double, geometry: RadarRouteProgress) {
        var position = driving.progress
        while position < distance {
            position = min(distance, position + 25)
            if let coordinate = geometry.coordinate(at: position) { driving.receive(fix(coordinate)) }
        }
        sync()
    }

    private func sync() {
        radarStatus = driving.radarMessage
        index = driving.nextStepIndex
        guard maneuvers.indices.contains(index) else { return }
        maneuver = maneuvers[index]
        nextManeuver = index + 1 < maneuvers.count ? maneuvers[index + 1] : nil
        let estimate = RadarManeuverFactory.currentEstimates(for: driving)
        distance = MKDistanceFormatter().string(fromDistance: estimate.distanceRemaining.converted(to: .meters).value)
        let remaining = RadarManeuverFactory.tripEstimates(for: driving)
        trip = "\(max(1, Int(ceil(remaining.timeRemaining / 60)))) min · \(MKDistanceFormatter().string(fromDistance: remaining.distanceRemaining.converted(to: .meters).value)) remaining"
    }

    private func capture(_ name: String) async throws {
        sync()
        // Wait for actual tile delivery rather than trusting the manifest label.
        if requiresRadar {
            for _ in 0..<30 {
                if loadedFrame == driving.frame, loadedTiles.values.contains(where: { $0 > 0 }) { break }
                if let coordinate = driving.location?.coordinate { driving.receive(fix(coordinate)) }
                try await Task.sleep(for: .seconds(1))
            }
            guard loadedFrame == driving.frame, loadedTiles.values.contains(where: { $0 > 0 }) else {
                throw ReviewError("No radar echoes delivered to the map; choose a location/scale with actual returns")
            }
        }
        try await Task.sleep(for: .seconds(5))
        guard let maneuver else { throw ReviewError("No current maneuver") }
        let estimates = RadarManeuverFactory.currentEstimates(for: driving)
        records.append([
            "file": name + ".png", "stepIndex": index,
            "instructionVariants": maneuver.instructionVariants,
            "dashboardInstructionVariants": maneuver.dashboardInstructionVariants,
            "symbolImagePresent": maneuver.symbolImage != nil,
            "initialDistanceMeters": maneuver.initialTravelEstimates!.distanceRemaining.converted(to: .meters).value,
            "initialTimeSeconds": maneuver.initialTravelEstimates!.timeRemaining,
            "updatedDistanceMeters": estimates.distanceRemaining.converted(to: .meters).value,
            "updatedTimeSeconds": estimates.timeRemaining,
            "tripDistanceMeters": driving.remainingDistance, "tripTimeSeconds": driving.remainingTime,
            "nextInstruction": nextManeuver?.instructionVariants.first ?? "",
            "radarFrameTimestamp": loadedFrame?.timestamp ?? "",
            "radarFramePath": loadedFrame?.path ?? "",
            "radarTileNontransparentPixels": loadedTiles,
            "radarTilesDelivered": loadedTiles.count,
            "radarTilesWithEchoes": loadedTiles.values.filter { $0 > 0 }.count,
        ])
        // Ask the runner to capture the visible device framebuffer rather
        // than rendering a separate offscreen representation.
        try name.write(to: directory.appendingPathComponent("capture-ready.txt"), atomically: true, encoding: .utf8)
        let acknowledgement = directory.appendingPathComponent("captured-" + name)
        for _ in 0..<45 {
            if FileManager.default.fileExists(atPath: acknowledgement.path) { return }
            if let coordinate = driving.location?.coordinate { driving.receive(fix(coordinate)) }
            try await Task.sleep(for: .seconds(1))
        }
        throw ReviewError("Timed out waiting for native screenshot")

    }

    func recordRadarTile(frame: RadarAPI.Frame, path: MKTileOverlayPath, data: Data) {
        guard frame == driving.frame, let image = UIImage(data: data)?.cgImage else { return }
        if loadedFrame != frame { loadedTiles.removeAll(); loadedFrame = frame }
        let width = image.width, height = image.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        let count: Int? = pixels.withUnsafeMutableBytes { bytes in
            guard let context = CGContext(data: bytes.baseAddress, width: width, height: height,
                bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return nil }
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return stride(from: 3, to: bytes.count, by: 4).reduce(0) { $0 + (bytes[$1] > 0 ? 1 : 0) }
        }
        if let count { loadedTiles["\(path.z)/\(path.x)/\(path.y)"] = count }
    }

    private struct ReviewError: LocalizedError {
        let errorDescription: String?
        init(_ message: String) { errorDescription = message }
    }
}

private struct ReviewMap: UIViewControllerRepresentable {
    let driving: RadarDrivingSession
    let coordinate: CLLocationCoordinate2D?
    let span: Double
    let onTileLoaded: (RadarAPI.Frame, MKTileOverlayPath, Data) -> Void
    func makeUIViewController(context: Context) -> RadarMapController {
        let controller = RadarMapController(dashboard: true, driving: driving)
        controller.onRadarTileLoaded = onTileLoaded
        controller.start()
        controller.mapView.showsUserLocation = false
        return controller
    }
    func updateUIViewController(_ controller: RadarMapController, context: Context) {
        guard let coordinate, driving.phase == .navigating else { return }
        controller.mapView.removeAnnotations(controller.mapView.annotations)
        let marker = MKPointAnnotation()
        marker.coordinate = coordinate
        marker.title = "Replayed GPS"
        controller.mapView.addAnnotation(marker)
        controller.mapView.setRegion(MKCoordinateRegion(center: coordinate,
            latitudinalMeters: span, longitudinalMeters: span), animated: false)
    }
    static func dismantleUIViewController(_ controller: RadarMapController, coordinator: ()) { controller.stop() }
}

private struct ReviewScreen: View {
    @StateObject private var model = ReviewModel()
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("RADAR NG").font(.headline)
                Spacer()
                Text("NAVIGATION REVIEW").font(.caption.bold()).foregroundStyle(.secondary)
            }
            Text(model.caption).font(.subheadline.bold())
            Text("\(model.city) · Live radar · Regional map").font(.caption).foregroundStyle(.secondary)
            HStack(spacing: 18) {
                if let image = model.maneuver?.symbolImage {
                    Image(uiImage: image).renderingMode(.template).resizable().scaledToFit().foregroundStyle(.white).frame(width: 48, height: 48)
                }
                VStack(alignment: .leading, spacing: 5) {
                    Text(model.distance).font(.largeTitle.bold())
                    Text(model.maneuver?.instructionVariants.first ?? "Calculating route…")
                        .font(.title3.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading).padding(16)
            .background(Color(red: 0.05, green: 0.25, blue: 0.18), in: RoundedRectangle(cornerRadius: 16))
            ReviewMap(driving: model.driving, coordinate: model.driving.location?.coordinate,
                span: model.mapSpan, onTileLoaded: model.recordRadarTile)
                .clipShape(RoundedRectangle(cornerRadius: 16))
            Text(model.trip).font(.headline)
            if let next = model.nextManeuver?.instructionVariants.first {
                Text("Then: \(next)").font(.subheadline).lineLimit(2)
            }
            Text(model.radarStatus).font(.caption).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 4) {
                Text("LIVE CPManeuver VALUES · STEP \(model.index + 1)").font(.caption2.bold())
                Text("Instruction + arrow: instructionVariants / symbolImage")
                Text("Distance: CPNavigationSession.updateEstimates payload")
                Text("Trip: CPMapTemplate.updateEstimates payload")
            }.font(.system(size: 10)).foregroundStyle(.secondary)
            Text("Development presentation · Replayed GPS\nCarPlay approval pending · Not CarPlay system UI")
                .font(.caption2).foregroundStyle(.secondary)
            if model.status.hasPrefix("FAIL") { Text(model.status).font(.caption).foregroundStyle(.red) }
        }
        .padding().preferredColorScheme(.dark).task { await model.run() }
    }
}
