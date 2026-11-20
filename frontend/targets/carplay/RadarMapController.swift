import CarPlay
import MapKit
import UIKit

@MainActor
final class RadarMapController: UIViewController {
    let mapView = MKMapView()
    let driving: RadarDrivingSession
    var onRadarTileLoaded: ((RadarFrame, MKTileOverlayPath, Data) -> Void)?
    private let dashboard: Bool
    private weak var interfaceController: CPInterfaceController?
    private var observer: UUID?
    private var radarOverlay: RadarTileOverlay?
    private var routeOverlay: MKPolyline?
    private var template: CPMapTemplate?
    private var buttonState: ButtonState?
    private var trip: CPTrip?
    private var navigation: CPNavigationSession?
    private var maneuvers: [CPManeuver] = []
    private var navigationRevision = -1
    private var navigationPaused = false
    private var followsLocation = true
    private var hasCentered = false
    private var radarVisible = true
    private var searchTask: Task<Void, Never>?
    private var routeTask: Task<Void, Never>?
    private var searchOperation: MKLocalSearch?
    private var searchCompletion: (([CPListItem]) -> Void)?
    private var searchGeneration = UUID()
    private var panOrigin: MKMapRect?
    private var zoomOrigin: MKMapRect?
    /// When panBeganWith stepped; a panWith right after it is the same press.
    private var arrowStepAt: Date?
    /// Outcome of each radar tile MapKit asked for; the status button reads the ones in view.
    private var tileStatuses: [RadarMapViewport.Tile: RadarTileStatus] = [:]
    private var wasNavigating = false

    init(dashboard: Bool = false, driving: RadarDrivingSession? = nil) {
        self.dashboard = dashboard
        self.driving = driving ?? .shared
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        mapView.frame = view.bounds
        mapView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        let configuration = MKStandardMapConfiguration(elevationStyle: .flat, emphasisStyle: .muted)
        configuration.pointOfInterestFilter = .excludingAll
        mapView.preferredConfiguration = configuration
        mapView.showsUserLocation = true
        mapView.showsCompass = false
        mapView.delegate = self
        // CarPlay owns all controls; the base view contains only map content.
        view.addSubview(mapView)
    }

    func makeTemplate(interfaceController: CPInterfaceController) -> CPMapTemplate {
        self.interfaceController = interfaceController
        let result = CPMapTemplate()
        template = result
        result.mapDelegate = self
        // Keep Search, radar status and the panning Done reachable; updateButtons hides it only while navigating.
        result.automaticallyHidesNavigationBar = false
        result.hidesButtonsWithNavigationBar = false
        result.mapButtons = [
            mapButton("location.fill") { [weak self] in self?.recenter() },
            mapButton("plus") { [weak self] in self?.zoom(levels: RadarMapViewport.zoomStep) },
            mapButton("minus") { [weak self] in self?.zoom(levels: -RadarMapViewport.zoomStep) },
            mapButton("hand.draw") { [weak self] in self?.template?.showPanningInterface(animated: true) },
        ]
        buttonState = nil  // a new template has no buttons yet
        updateButtons()
        return result
    }

    private func mapButton(_ symbol: String, action: @escaping () -> Void) -> CPMapButton {
        let button = CPMapButton { _ in action() }
        button.image = UIImage(systemName: symbol)
        return button
    }

    func start() {
        loadViewIfNeeded()
        guard observer == nil else { return }
        observer = driving.observe { [weak self] in self?.sync() }
    }

    func stop() {
        cancelSearch()
        routeTask?.cancel(); routeTask = nil
        if !dashboard { driving.endNavigation() }
        if let observer { driving.removeObserver(observer) }
        observer = nil
        navigation?.cancelTrip(); navigation = nil
    }

    private func sync() {
        if let frame = driving.frame, radarVisible, frame.isFresh() {
            if radarOverlay?.frame != frame {
                if let radarOverlay { mapView.removeOverlay(radarOverlay) }
                let overlay = RadarTileOverlay(frame: frame)
                overlay.onFailure = { [weak driving] in driving?.tileFailed(for: frame) }
                overlay.onTileLoaded = { [weak self] path, data in
                    self?.onRadarTileLoaded?(frame, path, data)
                }
                overlay.onTileStatus = { [weak self] path, status in self?.record(path, status) }
                radarOverlay = overlay
                mapView.insertOverlay(overlay, at: 0, level: .aboveRoads)
            }
        } else if let radarOverlay {
            mapView.removeOverlay(radarOverlay); self.radarOverlay = nil
        }
        if routeOverlay !== driving.selectedRoute?.polyline {
            if let routeOverlay { mapView.removeOverlay(routeOverlay) }
            routeOverlay = driving.selectedRoute?.polyline
            if let routeOverlay { mapView.addOverlay(routeOverlay, level: .aboveRoads) }
            if driving.phase == .preview, let routeOverlay {
                followsLocation = false
                mapView.setVisibleMapRect(routeOverlay.boundingMapRect,
                    edgePadding: UIEdgeInsets(top: 90, left: 90, bottom: 100, right: 70), animated: true)
            }
        }
        // Follow only fixes navigation accepts, but center once on the first fix of any quality.
        let fix = driving.hasUsableLocation ? driving.location : (hasCentered ? nil : driving.displayLocation)
        if followsLocation, let location = fix {
            if driving.phase == .navigating && !wasNavigating {
                setSpan(RadarMapViewport.navigationSpanMeters, around: location.coordinate, animated: true)
                hasCentered = true
            } else if !hasCentered {
                setSpan(RadarMapViewport.resetSpanMeters(navigating: false, dashboard: dashboard),
                        around: location.coordinate, animated: false)
                hasCentered = true
            } else { mapView.setCenter(location.coordinate, animated: true) }
        }
        wasNavigating = driving.phase == .navigating
        updateNavigation()
        updateButtons()
    }

    /// The locate button: follow again and return to the default zoom (street span while navigating).
    func recenter() {
        followsLocation = true
        if template?.isPanningInterfaceVisible == true { template?.dismissPanningInterface(animated: true) }
        guard let location = driving.location ?? driving.displayLocation else { return }
        let span = RadarMapViewport.resetSpanMeters(navigating: driving.phase == .navigating, dashboard: dashboard)
        setSpan(span, around: location.coordinate, animated: true)
    }

    private func setSpan(_ meters: Double, around center: CLLocationCoordinate2D, animated: Bool) {
        mapView.setRegion(MKCoordinateRegion(center: center, latitudinalMeters: meters, longitudinalMeters: meters),
                          animated: animated)
    }

    private func latitude(of rect: MKMapRect) -> Double { MKMapPoint(x: rect.midX, y: rect.midY).coordinate.latitude }

    private func widthMeters(of rect: MKMapRect) -> Double {
        rect.width * RadarMapViewport.metersPerMapPoint(latitude: latitude(of: rect))
    }

    /// Shows `meters` across, clamped to the radar's zooms, keeping `anchor` fixed on screen.
    private func setWidth(_ meters: Double, anchor: MKMapPoint, in rect: MKMapRect, animated: Bool) {
        guard rect.width > 0 else { return }
        let lat = latitude(of: rect)
        let clamped = RadarMapViewport.clampedWidth(meters, viewWidthPoints: Double(mapView.bounds.width), latitude: lat)
        let ratio = clamped / RadarMapViewport.metersPerMapPoint(latitude: lat) / rect.width
        mapView.setVisibleMapRect(MKMapRect(x: anchor.x - (anchor.x - rect.minX) * ratio,
                                            y: anchor.y - (anchor.y - rect.minY) * ratio,
                                            width: rect.width * ratio, height: rect.height * ratio), animated: animated)
    }

    private func zoom(levels: Double) {
        let rect = mapView.visibleMapRect
        setWidth(widthMeters(of: rect) / pow(2, levels), anchor: MKMapPoint(x: rect.midX, y: rect.midY),
                 in: rect, animated: true)
    }

    private func record(_ path: MKTileOverlayPath, _ status: RadarTileStatus) {
        if tileStatuses.count > 512 { tileStatuses.removeAll() }  // bound memory on long drives
        tileStatuses[RadarMapViewport.Tile(z: path.z, x: path.x, y: path.y)] = status
        updateButtons()
    }

    private func radarStatus() -> String {
        let rect = mapView.visibleMapRect
        let tiles = RadarMapViewport.visibleStatuses(tileStatuses, minX: rect.minX, minY: rect.minY,
                                                     maxX: rect.maxX, maxY: rect.maxY)
        return RadarMapViewport.statusText(frameAge: driving.frame?.observedAt.map { Date().timeIntervalSince($0) },
                                           fetchFailed: driving.radarFailed, hidden: !radarVisible, tiles: tiles)
    }

    /// What the bar buttons show. They are rebuilt only when this changes: publish() runs on every fix and tick.
    private struct ButtonState: Equatable {
        let phase: RadarDrivingSession.Phase
        let status: String
        let muted: Bool
        let panning: Bool
    }

    private func updateButtons() {
        guard let template else { return }
        let state = ButtonState(phase: driving.phase, status: driving.locationMessage ?? radarStatus(),
                                muted: driving.muted, panning: template.isPanningInterfaceVisible)
        guard state != buttonState else { return }
        buttonState = state
        template.automaticallyHidesNavigationBar = driving.phase == .navigating
        if template.isPanningInterfaceVisible {
            template.leadingNavigationBarButtons = [CPBarButton(title: "Done") { [weak self] _ in
                self?.template?.dismissPanningInterface(animated: true)
            }]
            return
        }
        let search = CPBarButton(title: "Search") { [weak self] _ in self?.showSearch() }
        if driving.phase == .navigating || driving.phase == .preview {
            template.leadingNavigationBarButtons = [search, CPBarButton(title: "End") { [weak self] _ in
                self?.template?.hideTripPreviews(); self?.driving.endNavigation()
            }]
        } else { template.leadingNavigationBarButtons = [search] }
        template.trailingNavigationBarButtons = [
            CPBarButton(title: state.status) { [weak self] _ in self?.showRadarStatus() },
            CPBarButton(image: UIImage(systemName: driving.muted ? "speaker.slash" : "speaker.wave.2")!) { [weak self] _ in
                self?.driving.muted.toggle()
            },
        ]
    }

    private func showRadarStatus() {
        let status = CPListItem(text: driving.radarMessage,
                               detailText: "Observed precipitation · Continental US · Updates every 2 minutes")
        status.isEnabled = false
        let refresh = CPListItem(text: "Refresh radar", detailText: nil)
        refresh.handler = { [weak self] _, done in self?.driving.refreshRadar(); done() }
        let toggle = CPListItem(text: radarVisible ? "Hide radar" : "Show radar", detailText: nil)
        toggle.handler = { [weak self] _, done in
            self?.radarVisible.toggle(); self?.sync()
            self?.interfaceController?.popTemplate(animated: true, completion: nil); done()
        }
        let list = CPListTemplate(title: "Radar", sections: [CPListSection(items: [status, refresh, toggle])])
        interfaceController?.pushTemplate(list, animated: true, completion: nil)
    }

    func showSearch() {
        let search = CPSearchTemplate()
        search.delegate = self
        interfaceController?.pushTemplate(search, animated: true, completion: nil)
    }

    private func cancelSearch() {
        searchGeneration = UUID()
        searchTask?.cancel(); searchTask = nil
        searchOperation?.cancel(); searchOperation = nil
        searchCompletion?([]); searchCompletion = nil
    }

    private func showError(_ error: Error) {
        let alert = CPAlertTemplate(titleVariants: [error.localizedDescription], actions: [
            CPAlertAction(title: "OK", style: .default) { [weak self] _ in
                self?.interfaceController?.dismissTemplate(animated: true, completion: nil)
            },
        ])
        interfaceController?.presentTemplate(alert, animated: true, completion: nil)
    }

    private func preview(_ destination: MKMapItem, completion: @escaping () -> Void) {
        routeTask?.cancel()
        routeTask = Task { [weak self] in
            defer { completion() }
            guard let self else { return }
            do {
                try await self.driving.preview(destination: destination)
                try Task.checkCancellation()
                self.interfaceController?.popToRootTemplate(animated: true, completion: nil)
                guard let trip = self.makeTrip() else { return }
                self.trip = trip
                self.template?.showTripPreviews([trip], textConfiguration: nil)
                self.template?.updateEstimates(self.tripEstimates(), for: trip)
            } catch {
                if !Task.isCancelled { self.showError(error) }
            }
        }
    }

    private func makeTrip() -> CPTrip? {
        guard let destination = driving.destination, let location = driving.location,
              let selected = driving.selectedRoute else { return nil }
        let candidates = driving.phase == .preview ? driving.routes : [selected]
        let choices = candidates.map { route in
            let formatter = MKDistanceFormatter()
            let minutes = max(1, Int(ceil(route.expectedTravelTime / 60)))
            let choice = CPRouteChoice(summaryVariants: ["\(minutes) min"],
                additionalInformationVariants: [formatter.string(fromDistance: route.distance)],
                selectionSummaryVariants: [route.name])
            choice.userInfo = route
            return choice
        }
        return CPTrip(origin: MKMapItem(location: location, address: nil),
                      destination: destination, routeChoices: choices)
    }

    private func tripEstimates() -> CPTravelEstimates {
        RadarManeuverFactory.tripEstimates(for: driving)
    }

    private func updateNavigation() {
        guard let template else { return }
        guard driving.phase == .navigating else {
            if driving.phase == .arrived { navigation?.finishTrip() }
            else { navigation?.cancelTrip() }
            navigation = nil; navigationRevision = -1; navigationPaused = false
            return
        }
        if navigationRevision != driving.routeRevision || (navigationPaused && driving.guidanceMessage == nil) {
            navigation?.cancelTrip()
            guard let trip = makeTrip() else { return }
            self.trip = trip
            template.hideTripPreviews()
            navigation = template.startNavigationSession(for: trip)
            navigationRevision = driving.routeRevision
            navigationPaused = false
            maneuvers = RadarManeuverFactory.makeManeuvers(for: driving)
            navigation?.add(maneuvers)
        }
        if let message = driving.guidanceMessage {
            navigation?.pauseTrip(for: driving.hasUsableLocation ? .rerouting : .locating, description: message)
            navigationPaused = true
            return
        }
        let index = driving.nextStepIndex
        if index < maneuvers.count {
            navigation?.upcomingManeuvers = Array(maneuvers[index..<min(index + 2, maneuvers.count)])
            navigation?.updateEstimates(RadarManeuverFactory.currentEstimates(for: driving),
                for: maneuvers[index])
        }
        if let trip { template.updateEstimates(tripEstimates(), for: trip) }
    }

}

extension RadarMapController: MKMapViewDelegate {
    func mapView(_ mapView: MKMapView, rendererFor overlay: MKOverlay) -> MKOverlayRenderer {
        if overlay is RadarTileOverlay {
            let renderer = MKTileOverlayRenderer(tileOverlay: overlay as! RadarTileOverlay)
            renderer.alpha = 0.8  // light echoes must read in sunlight over the muted base
            return renderer
        }
        if let route = overlay as? MKPolyline {
            let renderer = MKPolylineRenderer(polyline: route)
            renderer.strokeColor = .systemBlue; renderer.lineWidth = 7
            return renderer
        }
        return MKOverlayRenderer(overlay: overlay)
    }
    func mapView(_ mapView: MKMapView, regionDidChangeAnimated animated: Bool) { updateButtons() }
}

extension RadarMapController: CPSearchTemplateDelegate {
    func searchTemplate(_ searchTemplate: CPSearchTemplate, updatedSearchText searchText: String,
                        completionHandler: @escaping ([CPListItem]) -> Void) {
        cancelSearch()
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard query.count >= 2 else { completionHandler([]); return }
        searchCompletion = completionHandler
        let generation = searchGeneration
        searchTask = Task { [weak self] in
            guard let self else { return }
            do {
                try await Task.sleep(for: .milliseconds(300))
                let request = MKLocalSearch.Request()
                request.naturalLanguageQuery = query
                if let location = self.driving.location {
                    request.region = MKCoordinateRegion(center: location.coordinate,
                        latitudinalMeters: 100_000, longitudinalMeters: 100_000)
                }
                let search = MKLocalSearch(request: request)
                self.searchOperation = search
                let response = try await search.start()
                guard !Task.isCancelled, self.searchGeneration == generation else { return }
                let items = response.mapItems.prefix(10).map { place in
                    let item = CPListItem(text: place.name, detailText: place.addressRepresentations?.fullAddress(includingRegion: false, singleLine: true))
                    item.userInfo = place
                    return item
                }
                self.searchCompletion?(items); self.searchCompletion = nil
            } catch {
                guard self.searchGeneration == generation else { return }
                let item = CPListItem(text: "Search unavailable", detailText: "Check your connection and try again")
                item.isEnabled = false
                self.searchCompletion?([item]); self.searchCompletion = nil
            }
        }
    }

    func searchTemplate(_ searchTemplate: CPSearchTemplate, selectedResult item: CPListItem,
                        completionHandler: @escaping () -> Void) {
        guard let destination = item.userInfo as? MKMapItem else { completionHandler(); return }
        preview(destination, completion: completionHandler)
    }
}

extension RadarMapController: CPMapTemplateDelegate {
    func mapTemplate(_ mapTemplate: CPMapTemplate, selectedPreviewFor trip: CPTrip, using routeChoice: CPRouteChoice) {
        if let route = routeChoice.userInfo as? MKRoute { driving.select(route) }
        mapTemplate.updateEstimates(tripEstimates(), for: trip)
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, startedTrip trip: CPTrip, using routeChoice: CPRouteChoice) {
        if let route = routeChoice.userInfo as? MKRoute { driving.select(route) }
        do {
            try driving.startNavigation()
            followsLocation = true
            if let location = driving.location {
                setSpan(RadarMapViewport.navigationSpanMeters, around: location.coordinate, animated: true)
            }
        } catch { showError(error) }
    }
    func mapTemplateDidCancelNavigation(_ mapTemplate: CPMapTemplate) { driving.endNavigation() }
    func mapTemplate(_ mapTemplate: CPMapTemplate, shouldShowNotificationFor maneuver: CPManeuver) -> Bool { true }
    func mapTemplate(_ mapTemplate: CPMapTemplate, shouldUpdateNotificationFor maneuver: CPManeuver,
                     with travelEstimates: CPTravelEstimates) -> Bool { true }
    func mapTemplateDidShowPanningInterface(_ mapTemplate: CPMapTemplate) { followsLocation = false; updateButtons() }
    func mapTemplateDidDismissPanningInterface(_ mapTemplate: CPMapTemplate) { updateButtons() }
    private func step(_ direction: CPMapTemplate.PanDirection) {
        followsLocation = false
        let rect = mapView.visibleMapRect, step = RadarMapViewport.panStep
        var center = MKMapPoint(x: rect.midX, y: rect.midY)
        if direction.contains(.left) { center.x -= rect.width * step }
        if direction.contains(.right) { center.x += rect.width * step }
        if direction.contains(.up) { center.y -= rect.height * step }
        if direction.contains(.down) { center.y += rect.height * step }
        mapView.setCenter(center.coordinate, animated: true)
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, panWith direction: CPMapTemplate.PanDirection) {
        defer { arrowStepAt = nil }
        if let at = arrowStepAt, Date().timeIntervalSince(at) < 0.3 { return }  // panBeganWith already moved
        step(direction)
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, panBeganWith direction: CPMapTemplate.PanDirection) {
        step(direction); arrowStepAt = Date()
    }
    func mapTemplateDidBeginPanGesture(_ mapTemplate: CPMapTemplate) {
        followsLocation = false; panOrigin = mapView.visibleMapRect
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, didUpdatePanGestureWithTranslation translation: CGPoint, velocity: CGPoint) {
        guard let rect = panOrigin, mapView.bounds.width > 0, mapView.bounds.height > 0 else { return }
        // setCenter never changes the zoom, unlike refitting a rect each update.
        let center = MKMapPoint(x: rect.midX - translation.x / mapView.bounds.width * rect.width,
                                y: rect.midY - translation.y / mapView.bounds.height * rect.height)
        mapView.setCenter(center.coordinate, animated: false)
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, didEndPanGestureWithVelocity velocity: CGPoint) {
        panOrigin = nil
        let size = mapView.bounds.size
        guard size.width > 0, size.height > 0 else { return }
        let coast = RadarMapViewport.momentum(velocityX: Double(velocity.x), velocityY: Double(velocity.y),
                                              viewWidth: Double(size.width), viewHeight: Double(size.height))
        guard coast.dx != 0 || coast.dy != 0 else { return }
        let rect = mapView.visibleMapRect
        let target = MKMapPoint(x: rect.midX - coast.dx / Double(size.width) * rect.width,
                                y: rect.midY - coast.dy / Double(size.height) * rect.height)
        // A short ease-out reads as the map coasting to a stop.
        UIView.animate(withDuration: 0.5, delay: 0, options: [.curveEaseOut, .allowUserInteraction, .beginFromCurrentState]) {
            self.mapView.setCenter(target.coordinate, animated: true)
        }
    }
    // Touch zoom (pinch, double tap, two-finger double tap) arrives through these on iOS 26.
    func mapTemplateDidBeginZoomGesture(_ mapTemplate: CPMapTemplate) {
        followsLocation = false; zoomOrigin = mapView.visibleMapRect
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, didUpdateZoomGestureWithCenter center: CGPoint,
                     scale: CGFloat, velocity: CGFloat) {
        guard let rect = zoomOrigin, scale > 0, mapView.bounds.width > 0, mapView.bounds.height > 0 else { return }
        // Scale is relative to the gesture's start, so zoom from the starting rect about the fingers.
        let anchor = MKMapPoint(x: rect.minX + center.x / mapView.bounds.width * rect.width,
                                y: rect.minY + center.y / mapView.bounds.height * rect.height)
        setWidth(widthMeters(of: rect) / Double(scale), anchor: anchor, in: rect, animated: false)
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, didEndZoomGestureWithVelocity velocity: CGFloat) { zoomOrigin = nil }
}
