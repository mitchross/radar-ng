import CarPlay
import MapKit
import UIKit

final class RadarMapController: UIViewController {
    private let mapView = MKMapView()
    private let locationManager = RadarLocationManager()
    private var tileOverlay: RadarTileOverlay?
    private var refreshTimer: Timer?
    private var panelButton: CPBarButton?
    private var opacityLevel: Int = 70

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        mapView.frame = view.bounds
        mapView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        mapView.mapType = .mutedStandard
        mapView.showsCompass = false
        mapView.showsScale = false
        mapView.showsUserLocation = true
        mapView.delegate = self
        view.addSubview(mapView)
        addRadarOverlay()
    }

    func makeTemplate() -> CPMapTemplate {
        let template = CPMapTemplate()
        template.mapDelegate = self
        template.automaticallyHidesNavigationBar = false

        let recenter = CPBarButton(image: UIImage(systemName: "location.fill") ?? UIImage()) { [weak self] _ in
            self?.recenter()
        }
        let refresh = CPBarButton(image: UIImage(systemName: "arrow.clockwise") ?? UIImage()) { [weak self] _ in
            self?.reloadTiles()
        }
        let opacity = CPBarButton(image: UIImage(systemName: "slider.horizontal.3") ?? UIImage()) { [weak self] _ in
            self?.cycleOpacity()
        }
        template.leadingNavigationBarButtons = [recenter]
        template.trailingNavigationBarButtons = [refresh, opacity]
        self.panelButton = opacity
        return template
    }

    func start() {
        locationManager.onUpdate = { [weak self] coord in
            guard let self else { return }
            let region = MKCoordinateRegion(center: coord,
                                            latitudinalMeters: 180_000,
                                            longitudinalMeters: 180_000)
            self.mapView.setRegion(region, animated: true)
        }
        locationManager.start()
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 300, repeats: true) { [weak self] _ in
            self?.reloadTiles()
        }
    }

    func stop() {
        refreshTimer?.invalidate()
        refreshTimer = nil
        locationManager.stop()
    }

    private func addRadarOverlay() {
        let overlay = RadarTileOverlay()
        overlay.canReplaceMapContent = false
        overlay.opacity = CGFloat(opacityLevel) / 100.0
        tileOverlay = overlay
        mapView.addOverlay(overlay, level: .aboveLabels)
    }

    private func reloadTiles() {
        guard let overlay = tileOverlay else { return }
        mapView.removeOverlay(overlay)
        overlay.bumpCacheKey()
        mapView.addOverlay(overlay, level: .aboveLabels)
    }

    private func cycleOpacity() {
        let steps = [40, 60, 80, 100]
        let next = steps.first(where: { $0 > opacityLevel }) ?? steps[0]
        opacityLevel = next
        tileOverlay?.opacity = CGFloat(next) / 100.0
        reloadTiles()
    }

    private func recenter() {
        guard let coord = locationManager.lastCoordinate else { return }
        let region = MKCoordinateRegion(center: coord,
                                        latitudinalMeters: 180_000,
                                        longitudinalMeters: 180_000)
        mapView.setRegion(region, animated: true)
    }
}

extension RadarMapController: MKMapViewDelegate {
    func mapView(_ mapView: MKMapView, rendererFor overlay: MKOverlay) -> MKOverlayRenderer {
        if let tile = overlay as? RadarTileOverlay {
            let renderer = MKTileOverlayRenderer(tileOverlay: tile)
            renderer.alpha = tile.opacity
            return renderer
        }
        return MKOverlayRenderer(overlay: overlay)
    }
}

extension RadarMapController: CPMapTemplateDelegate {}
