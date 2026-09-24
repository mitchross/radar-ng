import CoreLocation

@MainActor
final class RadarLocationManager: NSObject, @preconcurrency CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    var onUpdate: ((CLLocation) -> Void)?
    var onUnavailable: ((String) -> Void)?
    private var running = false

    override init() {
        super.init()
        manager.delegate = self
        manager.activityType = .automotiveNavigation
        manager.desiredAccuracy = kCLLocationAccuracyBestForNavigation
        manager.distanceFilter = 10
    }

    func start() {
        running = true
        if manager.authorizationStatus == .notDetermined { manager.requestWhenInUseAuthorization() }
        locationManagerDidChangeAuthorization(manager)
    }

    func setNavigating(_ active: Bool) {
        let modes = Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String] ?? []
        manager.allowsBackgroundLocationUpdates = active && modes.contains("location")
        manager.pausesLocationUpdatesAutomatically = !active
    }

    func stop() {
        running = false
        setNavigating(false)
        manager.stopUpdatingLocation()
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        guard running else { return }
        switch manager.authorizationStatus {
        case .authorizedAlways, .authorizedWhenInUse: manager.startUpdatingLocation()
        case .denied, .restricted:
            manager.stopUpdatingLocation()
            onUnavailable?("Enable Radar NG location in iPhone Settings")
        default: onUnavailable?("Allow location in Radar NG on your iPhone")
        }
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let location = locations.last(where: { RadarRouteProgress.accepts($0) }) else { return }
        onUpdate?(location)
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        onUnavailable?("Location unavailable")
    }
}
