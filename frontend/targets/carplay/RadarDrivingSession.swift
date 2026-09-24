import AVFoundation
import CoreLocation
import MapKit

/// One journey and radar frame shared by the main CarPlay and Dashboard scenes.
@MainActor
final class RadarDrivingSession: NSObject, AVSpeechSynthesizerDelegate {
    static let shared = RadarDrivingSession()
    enum Phase { case browsing, preview, navigating, arrived }
    private(set) var phase = Phase.browsing
    private(set) var location: CLLocation?
    private(set) var locationMessage: String? = "Waiting for location"
    private(set) var frame: RadarAPI.Frame?
    private(set) var radarMessage = "Loading radar"
    private(set) var routes: [MKRoute] = []
    private(set) var selectedRoute: MKRoute?
    private(set) var destination: MKMapItem?
    private(set) var routeRevision = 0
    private(set) var progress: CLLocationDistance = 0
    private(set) var nextStepIndex = 0
    private(set) var guidanceMessage: String?
    private(set) var steps: [MKRoute.Step] = []
    private(set) var stepOffsets: [CLLocationDistance] = []
    var muted = false { didSet { if muted { stopSpeech() }; publish() } }

    var remainingDistance: CLLocationDistance { max(0, (geometry?.length ?? 0) - progress) }
    var remainingTime: TimeInterval {
        guard let route = selectedRoute, let geometry, geometry.length > 0 else { return 0 }
        return route.expectedTravelTime * remainingDistance / geometry.length
    }
    var maneuverDistance: CLLocationDistance {
        nextStepIndex < stepOffsets.count ? max(0, stepOffsets[nextStepIndex] - progress) : remainingDistance
    }
    var hasUsableLocation: Bool { location.map { RadarRouteProgress.accepts($0) } ?? false }

    private let locator = RadarLocationManager()
    private let speaker = AVSpeechSynthesizer()
    private var geometry: RadarRouteProgress?
    private var listeners: [UUID: () -> Void] = [:]
    private var timer: Timer?
    private var radarTask: Task<Void, Never>?
    private var routeTask: Task<Void, Never>?
    private var directions: MKDirections?
    private var routeRequest = UUID()
    private var lastRadarRefresh = Date.distantPast
    private var lastReroute = Date.distantPast
    private var lastFix: Date?
    private var offRouteFixes = 0
    private var spoken: Set<String> = []
    private let usesLiveLocation: Bool

    init(usesLiveLocation: Bool = true) {
        self.usesLiveLocation = usesLiveLocation
        super.init()
        speaker.delegate = self
        speaker.usesApplicationAudioSession = true
        locator.onUpdate = { [weak self] in self?.receive($0) }
        locator.onUnavailable = { [weak self] message in
            self?.locationMessage = message
            if self?.phase == .navigating { self?.guidanceMessage = message; self?.stopSpeech() }
            self?.publish()
        }
    }

    func observe(_ action: @escaping () -> Void) -> UUID {
        let id = UUID()
        listeners[id] = action
        if listeners.count == 1 {
            if usesLiveLocation { locator.start() }
            refreshRadar()
            timer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
                Task { @MainActor in self?.tick() }
            }
        }
        action()
        return id
    }

    func removeObserver(_ id: UUID) {
        listeners.removeValue(forKey: id)
        if listeners.isEmpty {
            endNavigation()
            locator.stop()
            timer?.invalidate(); timer = nil
            radarTask?.cancel(); radarTask = nil
        }
    }

    private func publish() { for action in Array(listeners.values) { action() } }

    private func tick() {
        if let frame, let date = frame.observedAt, Date().timeIntervalSince(date) > 900 {
            radarMessage = "Radar expired · \(date.formatted(date: .omitted, time: .shortened))"
        }
        if !hasUsableLocation {
            locationMessage = "Waiting for GPS"
            if phase == .navigating { guidanceMessage = "Waiting for GPS"; stopSpeech() }
        }
        if Date().timeIntervalSince(lastRadarRefresh) > 120 { refreshRadar() }
        publish()
    }

    func refreshRadar() {
        guard radarTask == nil else { return }
        lastRadarRefresh = .now
        radarTask = Task { [weak self] in
            guard let self else { return }
            defer { self.radarTask = nil }
            do {
                let next = try await RadarAPI.latestFrame()
                try Task.checkCancellation()
                self.frame = next
                let date = next.observedAt!
                self.radarMessage = "\(Date().timeIntervalSince(date) > 900 ? "Older radar" : "Radar") · \(date.formatted(date: .omitted, time: .shortened))"
            } catch {
                guard !Task.isCancelled else { return }
                self.frame = nil
                self.radarMessage = "Radar unavailable"
            }
            self.publish()
        }
    }

    func tileFailed(for frame: RadarAPI.Frame) {
        guard self.frame == frame else { return }
        self.frame = nil
        radarMessage = "Radar unavailable"
        publish()
    }

    func preview(destination: MKMapItem) async throws {
        guard hasUsableLocation, let location else { throw DrivingError.location }
        endNavigation()
        self.destination = destination
        let requestID = UUID()
        routeRequest = requestID
        let result = try await calculate(from: location, to: destination, alternatives: true)
        guard routeRequest == requestID else { throw CancellationError() }
        guard !result.isEmpty else { throw DrivingError.noRoute }
        routes = Array(result.prefix(3))
        phase = .preview
        select(routes[0])
    }

    private func calculate(from location: CLLocation, to destination: MKMapItem, alternatives: Bool) async throws -> [MKRoute] {
        directions?.cancel()
        let request = MKDirections.Request()
        request.source = MKMapItem(location: location, address: nil)
        request.destination = destination
        request.transportType = .automobile
        request.requestsAlternateRoutes = alternatives
        request.departureDate = .now
        let operation = MKDirections(request: request)
        directions = operation
        return try await withTaskCancellationHandler {
            try await operation.calculate().routes.filter { $0.polyline.pointCount > 1 && !$0.steps.isEmpty }
        } onCancel: { operation.cancel() }
    }

    func select(_ route: MKRoute) {
        selectedRoute = route
        geometry = RadarRouteProgress(polyline: route.polyline)
        progress = 0
        steps = []
        stepOffsets = []
        // MapKit's instruction describes the maneuver at the END of this
        // step's geometry (verified against the live route). For example, a
        // northbound segment ending at a westbound road says "Turn left".
        let offsets = geometry!.maneuverEndOffsets(for: route.steps.map(\.polyline))
        for (step, offset) in zip(route.steps, offsets) {
            if !step.instructions.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                steps.append(step)
                stepOffsets.append(offset)
            }
        }
        nextStepIndex = 0
        routeRevision += 1
        spoken.removeAll()
        offRouteFixes = 0
        publish()
    }

    func startNavigation() throws {
        guard hasUsableLocation else { throw DrivingError.location }
        guard selectedRoute != nil, destination != nil else { throw DrivingError.noRoute }
        phase = .navigating
        guidanceMessage = nil
        locator.setNavigating(true)
        if let location { receive(location) }
        publish()
    }

    func endNavigation(arrived: Bool = false) {
        routeRequest = UUID()
        directions?.cancel(); directions = nil
        routeTask?.cancel(); routeTask = nil
        stopSpeech()
        locator.setNavigating(false)
        phase = arrived ? .arrived : .browsing
        guidanceMessage = nil
        if !arrived {
            selectedRoute = nil; destination = nil; routes = []; geometry = nil
            steps = []; stepOffsets = []; progress = 0
        }
        publish()
    }

    func receive(_ fix: CLLocation) {
        guard RadarRouteProgress.accepts(fix) else { return }
        guard lastFix == nil || fix.timestamp >= lastFix! else { return }
        let elapsed = lastFix.map { max(0, fix.timestamp.timeIntervalSince($0)) } ?? 0
        lastFix = fix.timestamp
        location = fix; locationMessage = nil
        guard phase == .navigating, let geometry else { publish(); return }
        guard routeTask == nil else { publish(); return }
        let window = max(400, max(0, fix.speed) * min(elapsed, 30) * 2 + 100)
        guard let projected = geometry.project(fix.coordinate, near: progress, forwardWindow: window) else { return }
        let threshold = max(45, fix.horizontalAccuracy * 1.5)
        if projected.offRoute > threshold {
            offRouteFixes += 1
            guidanceMessage = "Locating route"
            stopSpeech()
            if offRouteFixes >= 3 && Date().timeIntervalSince(lastReroute) > 15 { reroute(from: fix) }
            publish()
            return
        }
        offRouteFixes = 0
        guidanceMessage = nil
        progress = max(progress, projected.distance)
        // Advance only after reaching a maneuver. Keep a short tolerance for GPS.
        while nextStepIndex + 1 < steps.count && stepOffsets[nextStepIndex] <= progress + 12 {
            nextStepIndex += 1
        }
        if remainingDistance < 25, let destination,
           fix.distance(from: destination.location) < 45 {
            endNavigation(arrived: true)
            speak("You have arrived at \(destination.name ?? "your destination")")
            return
        }
        announceManeuver()
        publish()
    }

    private func reroute(from fix: CLLocation) {
        guard let destination else { return }
        lastReroute = .now
        guidanceMessage = "Recalculating route"
        let requestID = UUID(); routeRequest = requestID
        routeTask = Task { [weak self] in
            guard let self else { return }
            defer { if self.routeRequest == requestID { self.routeTask = nil } }
            do {
                let routes = try await self.calculate(from: fix, to: destination, alternatives: false)
                guard !Task.isCancelled, self.routeRequest == requestID, self.phase == .navigating else { return }
                guard let route = routes.first else { throw DrivingError.noRoute }
                self.select(route)
                self.guidanceMessage = nil
                self.publish()
            } catch {
                guard !Task.isCancelled, self.routeRequest == requestID else { return }
                self.guidanceMessage = "Route unavailable · Retrying"
                self.publish()
            }
        }
    }

    private func announceManeuver() {
        guard nextStepIndex < steps.count, !muted, guidanceMessage == nil else { return }
        let distance = maneuverDistance
        let stage = distance < 50 ? "now" : distance < 350 ? "soon" : "ahead"
        let key = "\(routeRevision)-\(nextStepIndex)-\(stage)"
        guard !spoken.contains(key) else { return }
        spoken.insert(key)
        let instruction = steps[nextStepIndex].instructions
        let formatter = MKDistanceFormatter()
        speak(distance < 50 ? instruction : "In \(formatter.string(fromDistance: distance)), \(instruction)")
    }

    private func speak(_ text: String) {
        guard !muted, AVAudioSession.sharedInstance().promptStyle != .none else { return }
        stopSpeech()
        do {
            let audio = AVAudioSession.sharedInstance()
            try audio.setCategory(.playback, mode: .voicePrompt, options: [.duckOthers, .interruptSpokenAudioAndMixWithOthers])
            try audio.setActive(true)
            speaker.speak(AVSpeechUtterance(string: text))
        } catch { /* Visual guidance remains available if another audio session wins. */ }
    }

    private func stopSpeech() {
        speaker.stopSpeaking(at: .immediate)
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        Task { @MainActor in
            if !self.speaker.isSpeaking { try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation) }
        }
    }

    enum DrivingError: LocalizedError {
        case location, noRoute
        var errorDescription: String? {
            switch self {
            case .location: "A current location is needed. Enable Radar NG location on your iPhone."
            case .noRoute: "No driving route is available. Try another destination."
            }
        }
    }
}
