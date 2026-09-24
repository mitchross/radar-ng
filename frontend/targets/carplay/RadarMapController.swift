import CarPlay
import MapKit
import UIKit

@MainActor
final class RadarMapController: UIViewController {
    let mapView = MKMapView()
    let driving: RadarDrivingSession
    var onRadarTileLoaded: ((RadarAPI.Frame, MKTileOverlayPath, Data) -> Void)?
    private let dashboard: Bool
    private weak var interfaceController: CPInterfaceController?
    private var observer: UUID?
    private var radarOverlay: RadarTileOverlay?
    private var routeOverlay: MKPolyline?
    private var template: CPMapTemplate?
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
        result.automaticallyHidesNavigationBar = true
        result.hidesButtonsWithNavigationBar = false
        result.mapButtons = [
            mapButton("location.fill") { [weak self] in self?.recenter() },
            mapButton("plus") { [weak self] in self?.zoom(0.6) },
            mapButton("minus") { [weak self] in self?.zoom(1.6) },
            mapButton("hand.draw") { [weak self] in self?.template?.showPanningInterface(animated: true) },
        ]
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
        if let frame = driving.frame, radarVisible,
           Date().timeIntervalSince(frame.observedAt ?? .distantPast) <= 900 {
            if radarOverlay?.frame != frame {
                if let radarOverlay { mapView.removeOverlay(radarOverlay) }
                let overlay = RadarTileOverlay(frame: frame)
                overlay.onFailure = { [weak driving] in driving?.tileFailed(for: frame) }
                overlay.onTileLoaded = { [weak self] path, data in
                    self?.onRadarTileLoaded?(frame, path, data)
                }
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
        if followsLocation, let location = driving.location, driving.hasUsableLocation {
            if driving.phase == .navigating && !wasNavigating {
                mapView.setRegion(MKCoordinateRegion(center: location.coordinate,
                    latitudinalMeters: 2400, longitudinalMeters: 2400), animated: true)
                hasCentered = true
            } else if !hasCentered {
                mapView.setRegion(MKCoordinateRegion(center: location.coordinate,
                    latitudinalMeters: dashboard ? 140_000 : 180_000,
                    longitudinalMeters: dashboard ? 140_000 : 180_000), animated: false)
                hasCentered = true
            } else { mapView.setCenter(location.coordinate, animated: true) }
        }
        wasNavigating = driving.phase == .navigating
        updateNavigation()
        updateButtons()
    }

    func recenter() {
        followsLocation = true
        if let location = driving.location {
            mapView.setCenter(location.coordinate, animated: true)
        }
    }

    private func zoom(_ multiplier: Double) {
        let rect = mapView.visibleMapRect
        let width = min(MKMapSize.world.width, max(500, rect.width * multiplier))
        let ratio = width / rect.width
        let size = MKMapSize(width: width, height: rect.height * ratio)
        mapView.setVisibleMapRect(MKMapRect(x: rect.midX - size.width / 2, y: rect.midY - size.height / 2,
                                         width: size.width, height: size.height), animated: true)
    }

    private func updateButtons() {
        guard let template else { return }
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
        let status = driving.locationMessage ?? driving.radarMessage
        template.trailingNavigationBarButtons = [
            CPBarButton(title: status) { [weak self] _ in self?.showRadarStatus() },
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
            renderer.alpha = 0.65
            return renderer
        }
        if let route = overlay as? MKPolyline {
            let renderer = MKPolylineRenderer(polyline: route)
            renderer.strokeColor = .systemBlue; renderer.lineWidth = 7
            return renderer
        }
        return MKOverlayRenderer(overlay: overlay)
    }
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
                mapView.setRegion(MKCoordinateRegion(center: location.coordinate,
                    latitudinalMeters: 2400, longitudinalMeters: 2400), animated: true)
            }
        } catch { showError(error) }
    }
    func mapTemplateDidCancelNavigation(_ mapTemplate: CPMapTemplate) { driving.endNavigation() }
    func mapTemplate(_ mapTemplate: CPMapTemplate, shouldShowNotificationFor maneuver: CPManeuver) -> Bool { true }
    func mapTemplate(_ mapTemplate: CPMapTemplate, shouldUpdateNotificationFor maneuver: CPManeuver,
                     with travelEstimates: CPTravelEstimates) -> Bool { true }
    func mapTemplateDidShowPanningInterface(_ mapTemplate: CPMapTemplate) { followsLocation = false; updateButtons() }
    func mapTemplateDidDismissPanningInterface(_ mapTemplate: CPMapTemplate) { updateButtons() }
    func mapTemplate(_ mapTemplate: CPMapTemplate, panWith direction: CPMapTemplate.PanDirection) {
        followsLocation = false
        var rect = mapView.visibleMapRect
        if direction.contains(.left) { rect.origin.x -= rect.width * 0.3 }
        if direction.contains(.right) { rect.origin.x += rect.width * 0.3 }
        if direction.contains(.up) { rect.origin.y -= rect.height * 0.3 }
        if direction.contains(.down) { rect.origin.y += rect.height * 0.3 }
        mapView.setVisibleMapRect(rect, animated: true)
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, panBeganWith direction: CPMapTemplate.PanDirection) {
        self.mapTemplate(mapTemplate, panWith: direction)
    }
    func mapTemplateDidBeginPanGesture(_ mapTemplate: CPMapTemplate) {
        followsLocation = false; panOrigin = mapView.visibleMapRect
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, didUpdatePanGestureWithTranslation translation: CGPoint, velocity: CGPoint) {
        guard var rect = panOrigin, mapView.bounds.width > 0, mapView.bounds.height > 0 else { return }
        rect.origin.x -= translation.x / mapView.bounds.width * rect.width
        rect.origin.y -= translation.y / mapView.bounds.height * rect.height
        mapView.setVisibleMapRect(rect, animated: false)
    }
    func mapTemplate(_ mapTemplate: CPMapTemplate, didEndPanGestureWithVelocity velocity: CGPoint) { panOrigin = nil }
}
