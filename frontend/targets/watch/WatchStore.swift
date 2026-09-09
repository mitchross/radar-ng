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
    @Published var radarFrame: WatchRadarFrame?
    @Published var radarErrorMessage: String?
    @Published var isRadarLoading = false
    @Published var locationNotice: String?
    @Published var updatedAt: Date?

    private var requestedLocation = false
    private var pendingLocationRefresh = false
    private let clManager = CLLocationManager()
    private let fallback = CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681)

    override init() {
        super.init()
        clManager.delegate = self
        clManager.desiredAccuracy = kCLLocationAccuracyKilometer
    }

    func refresh() async {
        guard !isLoading else { return }
        isLoading = true
        isRadarLoading = true
        defer {
            isLoading = false
            isRadarLoading = false
            if pendingLocationRefresh {
                pendingLocationRefresh = false
                Task { await self.refresh() }
            }
        }
        if location == nil && !requestedLocation {
            requestedLocation = true
            switch clManager.authorizationStatus {
            case .authorizedAlways, .authorizedWhenInUse:
                clManager.requestLocation()
            case .notDetermined:
                clManager.requestWhenInUseAuthorization()
            default:
                locationNotice = "Showing Grand Rapids"
            }
        }
        let coord = location ?? fallback
        async let forecastResult = Self.capture { try await WatchAPI.fetchForecast(lat: coord.latitude, lon: coord.longitude) }
        async let alertResult = Self.capture { try await WatchAPI.fetchAlerts(lat: coord.latitude, lon: coord.longitude) }
        async let radarResult = Self.capture { try await WatchAPI.fetchLatestRadarFrame() }

        switch await forecastResult {
        case .success(let forecast):
            self.forecast = forecast
            self.errorMessage = nil
            self.updatedAt = Date()
        case .failure(let error):
            self.errorMessage = error.localizedDescription
        }
        if case .success(let alerts) = await alertResult {
            self.alerts = alerts
        }
        apply(await radarResult)
    }

    func refreshRadar() async {
        guard !isRadarLoading else { return }
        isRadarLoading = true
        defer { isRadarLoading = false }
        apply(await Self.capture { try await WatchAPI.fetchLatestRadarFrame() })
    }

    private func apply(_ result: Result<WatchRadarFrame, Error>) {
        switch result {
        case .success(let frame):
            radarFrame = frame
            radarErrorMessage = nil
        case .failure(let error):
            radarErrorMessage = error.localizedDescription
        }
    }

    private static func capture<T>(
        _ operation: () async throws -> T
    ) async -> Result<T, Error> {
        do {
            return .success(try await operation())
        } catch {
            return .failure(error)
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let coord = locations.last?.coordinate else { return }
        Task { @MainActor in
            let previous = self.location
            self.location = coord
            self.locationNotice = nil
            if previous == nil || CLLocation(latitude: previous!.latitude, longitude: previous!.longitude)
                .distance(from: CLLocation(latitude: coord.latitude, longitude: coord.longitude)) > 2000 {
                if self.isLoading {
                    self.pendingLocationRefresh = true
                } else {
                    await self.refresh()
                }
            }
        }
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        let status = manager.authorizationStatus
        Task { @MainActor in
            guard self.requestedLocation else { return }
            if status == .authorizedAlways || status == .authorizedWhenInUse {
                if self.location == nil { self.clManager.requestLocation() }
            } else if status == .denied || status == .restricted {
                self.locationNotice = "Showing Grand Rapids"
            }
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        Task { @MainActor in
            // A denied/failed location must never recursively trigger more requests.
            self.locationNotice = "Showing Grand Rapids"
        }
    }
}
