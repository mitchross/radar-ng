import CoreLocation

final class RadarLocationManager: NSObject, CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    var onUpdate: ((CLLocationCoordinate2D) -> Void)?
    private(set) var lastCoordinate: CLLocationCoordinate2D?

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyKilometer
        manager.distanceFilter = 2000
    }

    func start() {
        manager.requestWhenInUseAuthorization()
        manager.startUpdatingLocation()
    }

    func stop() {
        manager.stopUpdatingLocation()
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let coord = locations.last?.coordinate else { return }
        lastCoordinate = coord
        onUpdate?(coord)
    }
}
