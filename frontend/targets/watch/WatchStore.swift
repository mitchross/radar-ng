import Foundation
import CoreLocation
import Combine

@MainActor
final class WatchStore: NSObject, ObservableObject, CLLocationManagerDelegate {
    @Published var forecast: Forecast?
    @Published var alerts: [Alert] = []
    @Published var isLoading = false
    @Published var errorMessage: String?
    @Published var location: CLLocationCoordinate2D?

    private let clManager = CLLocationManager()
    private let fallback = CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681)

    override init() {
        super.init()
        clManager.delegate = self
        clManager.desiredAccuracy = kCLLocationAccuracyKilometer
    }

    func refresh() async {
        isLoading = true
        defer { isLoading = false }
        if location == nil {
            clManager.requestWhenInUseAuthorization()
            clManager.requestLocation()
        }
        let coord = location ?? fallback
        do {
            async let f = WatchAPI.fetchForecast(lat: coord.latitude, lon: coord.longitude)
            async let a = WatchAPI.fetchAlerts(lat: coord.latitude, lon: coord.longitude)
            let (forecast, alerts) = try await (f, a)
            self.forecast = forecast
            self.alerts = alerts
            self.errorMessage = nil
        } catch {
            self.errorMessage = error.localizedDescription
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let coord = locations.last?.coordinate else { return }
        Task { @MainActor in
            self.location = coord
            await self.refresh()
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        Task { @MainActor in await self.refresh() }
    }
}
